enable wgpu_binding_array;

struct CameraUniforms {
    camera_pos: vec4<f32>,
    view_inv: mat4x4<f32>,
    proj_inv: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    viewport_and_heatmap: vec4<f32>,
};

struct VertexInput {
    @location(0) position: vec4<f32>,
    @location(1) tex_coord: vec2<f32>,
};

struct InstanceInput {
    @location(2) model_matrix_0: vec4<f32>,
    @location(3) model_matrix_1: vec4<f32>,
    @location(4) model_matrix_2: vec4<f32>,
    @location(5) model_matrix_3: vec4<f32>,
    @location(6) chunk_origin_in: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
    @location(1) @interpolate(flat) chunk_id: u32,
    @location(2) @interpolate(flat) chunk_origin: vec3<f32>,
    @location(3) world_position: vec3<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(1) @binding(0) var<storage, read> palette: array<vec4<f32>>;
@group(1) @binding(1) var voxel_textures: binding_array<texture_3d<u32>>;

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

// --- Chunk DDA constants (phase 1: single coarse level) -------------------
//
// The chunk is a 256^3 volume; 1 voxel = VOXEL_SCALE (0.125 m), so a chunk spans
// 32 m. The 3D texture has 9 mip levels (256 -> 128 -> ... -> 1). Phase 1
// marches a single coarse level and samples occupancy directly.
//
// CHUNK_WORLD_SIZE / GRID_SIZE / CELL_SIZE are hardcoded here and MUST match
// the CPU source of truth: CHUNK_TEXTURE_SIZE.width * VOXEL_SCALE (32.0).
const CHUNK_WORLD_SIZE: f32 = 32.0;
const MIP_LEVEL: u32 = 5u;       // 256 >> 5 = 8 cells per axis
const GRID_SIZE: i32 = 8;         // 256 >> MIP_LEVEL
const CELL_SIZE: f32 = 4.0;       // CHUNK_WORLD_SIZE / GRID_SIZE (metres)
const MAX_STEPS: i32 = 24;        // > diagonal of 8^3 (8*sqrt(3) ~ 14) headroom
const INF: f32 = 1.0e30;
const PARALLEL_EPS: f32 = 1.0e-8; // below this |dir component| the ray is parallel to the slab

@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );

    let world_position = model_matrix * model.position;

    var out: VertexOutput;

    out.position = camera.view_proj * world_position;
    out.tex_coord = model.tex_coord;
    out.chunk_id = instance_index;
    out.chunk_origin = instance.chunk_origin_in.xyz;
    out.world_position = world_position.xyz;

    return out;
}

// Returns vec2(t_enter, t_exit) for the ray vs chunk AABB. `t_enter` is clamped
// to >= 0 so a camera inside the volume starts marching at the camera itself.
// Miss (no overlap, or AABB entirely behind) is signalled by t_exit < 0.
fn ray_aabb(origin: vec3<f32>, dir: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>) -> vec2<f32> {
    var t_enter: f32 = 0.0;
    var t_exit: f32 = INF;

    for (var i: i32 = 0; i < 3; i++) {
        let o = origin[i];
        let d = dir[i];
        let lo = bmin[i];
        let hi = bmax[i];
        if (abs(d) < PARALLEL_EPS) {
            // Ray parallel to this slab: hit only if origin is inside it.
            if (o < lo || o > hi) {
                return vec2<f32>(-1.0, -1.0);
            }
            continue;
        }
        let inv = 1.0 / d;
        let t1 = (lo - o) * inv;
        let t2 = (hi - o) * inv;
        var tlo = min(t1, t2);
        var thi = max(t1, t2);
        t_enter = max(t_enter, tlo);
        t_exit = min(t_exit, thi);
    }

    if (t_exit < t_enter || t_exit < 0.0) {
        return vec2<f32>(-1.0, -1.0);
    }
    return vec2<f32>(max(t_enter, 0.0), t_exit);
}

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    let chunk_origin = in.chunk_origin;
    let bmin = chunk_origin;
    let bmax = chunk_origin + vec3<f32>(CHUNK_WORLD_SIZE);

    let origin = camera.camera_pos.xyz;
    let dir = normalize(in.world_position - origin);

    let span = ray_aabb(origin, dir, bmin, bmax);
    if (span[1] < 0.0) {
        discard;
    }
    let t_start = span[0];
    let t_exit = span[1];

    // Amanatides-Woo setup in world space. `t` is distance in metres along the
    // (unit) ray; cell indices are recovered from the world hit point.
    let start = origin + dir * t_start;
    var cell = vec3<i32>(floor((start - chunk_origin) / vec3<f32>(CELL_SIZE)));

    var t_max = vec3<f32>(INF);
    var t_delta = vec3<f32>(INF);
    var step = vec3<i32>(0);

    for (var i: i32 = 0; i < 3; i++) {
        let d = dir[i];
        if (abs(d) < PARALLEL_EPS) {
            t_max[i] = INF;
            t_delta[i] = INF;
            step[i] = 0;
        } else {
            let inv = abs(1.0 / d);
            t_delta[i] = CELL_SIZE * inv;
            step[i] = select(-1, 1, d > 0.0);
            // Distance to the next boundary along this axis.
            let boundary = chunk_origin[i] + f32(select(cell[i], cell[i] + 1, d > 0.0)) * CELL_SIZE;
            t_max[i] = (boundary - origin[i]) / d;
        }
    }

    var t: f32 = t_start;
    for (var i: i32 = 0; i < MAX_STEPS; i = i + 1) {
        // Only sample cells that fall inside the grid; out-of-range cells are empty.
        if (all(cell >= vec3<i32>(0)) && all(cell < vec3<i32>(GRID_SIZE))) {
            let mat = textureLoad(voxel_textures[in.chunk_id], cell, MIP_LEVEL).x;
            if (mat != 0u) {
                let hit_world = origin + dir * t;
                let clip = camera.view_proj * vec4<f32>(hit_world, 1.0);
                return FragmentOutput(palette[mat], clip.z / clip.w);
            }
        }

        // Step to the nearest next cell boundary.
        if (t_max[0] <= t_max[1] && t_max[0] <= t_max[2]) {
            t = t_max[0];
            t_max[0] = t_max[0] + t_delta[0];
            cell[0] = cell[0] + step[0];
        } else if (t_max[1] <= t_max[2]) {
            t = t_max[1];
            t_max[1] = t_max[1] + t_delta[1];
            cell[1] = cell[1] + step[1];
        } else {
            t = t_max[2];
            t_max[2] = t_max[2] + t_delta[2];
            cell[2] = cell[2] + step[2];
        }

        if (t > t_exit) {
            break;
        }
    }

    // Ray left the chunk without hitting a non-empty cell.
    discard;
}
