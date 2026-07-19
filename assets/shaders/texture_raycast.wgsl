struct Camera {
    pos: vec4<f32>,
    view_inv: mat4x4<f32>,
    proj_inv: mat4x4<f32>,
    heatmap: u32,
}

@binding(0) @group(0) var output : texture_storage_2d<rgba8unorm, write>;
@binding(1) @group(0) var<uniform> camera : Camera;
@binding(2) @group(0) var voxel_data : texture_3d<u32>;
@binding(3) @group(0) var<storage, read> palette : array<vec4<f32>>;

const VOLUME_SIZE: u32 = 1024u;
const VOLUME_SIZE_F: f32 = 1024.0;
const INF: f32 = 1e30;

const MAX_STEPS: i32 = 512;
const MAX_MIP_LEVEL: i32 = 4;

@compute
@workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let pixel = global_id.xy;
    let dims = vec2<f32>(textureDimensions(output));

    if pixel.x >= u32(dims.x) || pixel.y >= u32(dims.y) {
        return;
    }

    let uv = (vec2<f32>(pixel) + 0.5) / dims;
    let ndc = uv * 2.0 - 1.0;

    let _ray_origin = camera.pos.xyz;
    let ray_origin = vec3<f32>(_ray_origin.x, _ray_origin.z, _ray_origin.y);

    let clip_pos = vec4<f32>(ndc.x, ndc.y, 1.0, 1.0);
    let view_pos = camera.proj_inv * clip_pos;
    let view_dir = view_pos.xyz / view_pos.w;
    let _ray_dir = normalize((camera.view_inv * vec4<f32>(view_dir, 0.0)).xyz);

    let ray_dir = vec3<f32>(_ray_dir.x, _ray_dir.z, _ray_dir.y);

    let inv_dir = 1.0 / ray_dir;

    let t0 = (vec3<f32>(0.0) - ray_origin) * inv_dir;
    let t1 = (vec3<f32>(VOLUME_SIZE_F) - ray_origin) * inv_dir;


    let t_min = min(t0, t1);
    let t_max = max(t0, t1);

    let t_near = max(max(t_min.x, t_min.y), max(t_min.z, 0.0));
    let t_far  = min(min(t_max.x, t_max.y), t_max.z);

    var color = vec4<f32>(ray_dir.x, ray_dir.y, ray_dir.z, 0.0);

    if t_near > t_far {
        textureStore(output, pixel, color);
        return;
    }

    let stepX = select(-1, 1, ray_dir.x > 0.0);
    let stepY = select(-1, 1, ray_dir.y > 0.0);
    let stepZ = select(-1, 1, ray_dir.z > 0.0);

    var hit           = false;
    var hit_voxel_id  = 0u;
    var hit_normal    = vec3<f32>(0.0);
    var steps_taken   = 0u;
    var t_entry       = t_near;   // entry t of current region
    var entry_bias    = 1e-4;
    var steps_remaining = MAX_STEPS;

    // Bound stack: parent_exit[L] is the exit t of the level-(L+1) voxel
    // that bounds the search at level L.  The coarsest level is bounded
    // by the volume exit (t_far).  This lets a false-positive descent
    // ascend and resume the parent DDA within the grandparent's bounds.
    var parent_exit: array<f32, 5>;
    parent_exit[0] = t_far;
    parent_exit[1] = t_far;
    parent_exit[2] = t_far;
    parent_exit[3] = t_far;
    parent_exit[4] = t_far;

    var level = MAX_MIP_LEVEL;

    while level >= 0 {
        let voxel_size = f32(1u << u32(level));
        let grid_size  = i32(VOLUME_SIZE >> u32(level));
        let mip        = level;
        let t_bound    = parent_exit[level];

        // Bias forward to avoid landing exactly on a voxel boundary.
        let pos = ray_origin + ray_dir * (t_entry + entry_bias);
        var X = i32(clamp(floor(pos.x / voxel_size), 0.0, f32(grid_size - 1)));
        var Y = i32(clamp(floor(pos.y / voxel_size), 0.0, f32(grid_size - 1)));
        var Z = i32(clamp(floor(pos.z / voxel_size), 0.0, f32(grid_size - 1)));

        // tMax: distance to next voxel boundary along each axis.
        let bX = voxel_size * f32(select(X, X + 1, ray_dir.x > 0.0));
        let bY = voxel_size * f32(select(Y, Y + 1, ray_dir.y > 0.0));
        let bZ = voxel_size * f32(select(Z, Z + 1, ray_dir.z > 0.0));

        var tMaxX: f32;
        var tMaxY: f32;
        var tMaxZ: f32;
        if ray_dir.x != 0.0 {
            tMaxX = (bX - ray_origin.x) / ray_dir.x;
        } else {
            tMaxX = INF;
        }
        if ray_dir.y != 0.0 {
            tMaxY = (bY - ray_origin.y) / ray_dir.y;
        } else {
            tMaxY = INF;
        }
        if ray_dir.z != 0.0 {
            tMaxZ = (bZ - ray_origin.z) / ray_dir.z;
        } else {
            tMaxZ = INF;
        }

        // tDelta: t to cross one full voxel along each axis.
        let tDeltaX = voxel_size / abs(ray_dir.x);
        let tDeltaY = voxel_size / abs(ray_dir.y);
        let tDeltaZ = voxel_size / abs(ray_dir.z);

        let start_voxel = textureLoad(voxel_data, vec3<i32>(X, Y, Z), mip).r;
        steps_taken++;
        if start_voxel != 0u {
            if level == 0 {
                hit = true;
                hit_voxel_id = start_voxel;
                // Gradient normal for the very first voxel we land in.
                let nx = f32(textureLoad(voxel_data, vec3<i32>(X + 1, Y,     Z    ), 0).r)
                       - f32(textureLoad(voxel_data, vec3<i32>(X - 1, Y,     Z    ), 0).r);
                let ny = f32(textureLoad(voxel_data, vec3<i32>(X,     Y + 1, Z    ), 0).r)
                       - f32(textureLoad(voxel_data, vec3<i32>(X,     Y - 1, Z    ), 0).r);
                let nz = f32(textureLoad(voxel_data, vec3<i32>(X,     Y,     Z + 1), 0).r)
                       - f32(textureLoad(voxel_data, vec3<i32>(X,     Y,     Z - 1), 0).r);
                let grad = vec3<f32>(-nx, -ny, -nz);
                hit_normal = select(vec3<f32>(0.0, 1.0, 0.0), normalize(grad), length(grad) > 1e-6);
                break;
            }
            // Coarse starting voxel is filled: descend into it.  t_entry
            // stays as-is (we entered this region at the same point);
            // bound the finer level to this voxel's exit.
            parent_exit[level - 1] = min(tMaxX, min(tMaxY, tMaxZ));
            level = level - 1;
            continue;
        }

        var stepAxis:    i32 = -1;
        var lastStepDir: i32 =  0;
        var found_coarse = false;

        loop {
            if steps_remaining <= 0 { break; }
            steps_remaining--;

            // t at which we cross out of the current voxel.
            let t_cross = min(tMaxX, min(tMaxY, tMaxZ));

            // Past the parent voxel's exit boundary?  This region held no
            // hit — it was a max-pool false positive.
            if t_cross >= t_bound { break; }

            // Advance along the axis whose boundary is closest.
            if tMaxX < tMaxY {
                if tMaxX < tMaxZ {
                    X += stepX;
                    if X < 0 || X >= grid_size { break; }
                    tMaxX += tDeltaX;
                    stepAxis = 0;
                    lastStepDir = stepX;
                } else {
                    Z += stepZ;
                    if Z < 0 || Z >= grid_size { break; }
                    tMaxZ += tDeltaZ;
                    stepAxis = 2;
                    lastStepDir = stepZ;
                }
            } else {
                if tMaxY < tMaxZ {
                    Y += stepY;
                    if Y < 0 || Y >= grid_size { break; }
                    tMaxY += tDeltaY;
                    stepAxis = 1;
                    lastStepDir = stepY;
                } else {
                    Z += stepZ;
                    if Z < 0 || Z >= grid_size { break; }
                    tMaxZ += tDeltaZ;
                    stepAxis = 2;
                    lastStepDir = stepZ;
                }
            }

            let voxel = textureLoad(voxel_data, vec3<i32>(X, Y, Z), mip).r;
            steps_taken++;
            if voxel != 0u {
                if level == 0 {
                    hit = true;
                    hit_voxel_id = voxel;
                    if stepAxis == 0 {
                        hit_normal = vec3<f32>(f32(-lastStepDir), 0.0, 0.0);
                    } else if stepAxis == 1 {
                        hit_normal = vec3<f32>(0.0, f32(-lastStepDir), 0.0);
                    } else {
                        hit_normal = vec3<f32>(0.0, 0.0, f32(-lastStepDir));
                    }
                    break;
                }
                // Coarse hit: record bounds and descend.
                t_entry = t_cross;
                // The DDA state has already advanced into this coarse voxel;
                // its next boundary is the voxel's exit.
                parent_exit[level - 1] = min(tMaxX, min(tMaxY, tMaxZ));
                found_coarse = true;
                level = level - 1;
                break;
            }
        }

        if hit { break; }
        if steps_remaining <= 0 { break; }  // budget exhausted → miss
        if found_coarse { continue; }       // descend to finer level

        // DDA exited the parent region without a hit.  If we're not at the
        // coarsest level, the parent was a max-pool false positive:
        // ascend one level and resume the coarser DDA from this region's
        // exit boundary (within the grandparent's bounds).
        if level < MAX_MIP_LEVEL {
            t_entry = t_bound;
            // Move beyond the boundary when reconstructing the parent DDA,
            // avoiding re-entry into the cell just left for negative rays.
            entry_bias = 1e-2;
            level = level + 1;
            continue;
        }

        // Coarsest level exhausted → genuine miss.
        break;
    }

    if hit {
        if camera.heatmap != 0u {
            // Blue → cyan → yellow → red, with more expensive rays hotter.
            let amount = clamp(f32(steps_taken) / f32(MAX_STEPS), 0.0, 1.0);
            let heat = clamp(amount * 4.0, 0.0, 4.0);
            let red = clamp(heat - 2.0, 0.0, 1.0);
            let green = 1.0 - abs(heat - 2.0) * 0.5;
            let blue = clamp(1.0 - heat, 0.0, 1.0);
            color = vec4<f32>(vec3<f32>(red, green, blue), 1.0);
        } else {
            let base_rgb = palette[hit_voxel_id].rgb;

            let light_dir = normalize(vec3<f32>(0.5, 1.0, 0.3));
            let ambient = 0.15;
            let diffuse = max(dot(hit_normal, light_dir), 0.0);

            color = vec4<f32>(base_rgb * (ambient + diffuse * 0.85), 1.0);
        }
    }

    textureStore(output, pixel, color);
}
