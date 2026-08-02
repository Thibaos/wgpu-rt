enable wgpu_binding_array;
enable wgpu_ray_query;

// %%STATS_DECLS%%

struct CameraUniforms {
    camera_pos: vec4<f32>,
    view_inv: mat4x4<f32>,
    proj_inv: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    viewport_and_heatmap: vec4<f32>,
};

// One chunk-world AABB primitive (32 bytes: six f32 + two padding f32,
// matching GpuAabb in src/render/mod.rs). Written by the CPU into the BLAS
// input buffer; also read back here to recover the hit chunk's world bounds
// for the DDA (upstream `ray_aabb_compute` pattern: primitive_offset = i * 32,
// so TLAS instance i's AABB is gpu_aabbs[i]). Scalar fields keep the WGSL
// array stride at 32 bytes (vec3 members would align to 16 and stride the
// array at 48, desyncing from the CPU buffer).
struct GpuAabb {
    min_x: f32,
    min_y: f32,
    min_z: f32,
    max_x: f32,
    max_y: f32,
    max_z: f32,
    _pad0: f32,
    _pad1: f32,
};

struct HitResult {
    t: f32,
    mat: u32,
};

@group(0) @binding(0) var<uniform> camera: CameraUniforms;

@group(1) @binding(0) var<storage, read> palette: array<vec4<f32>>;
@group(1) @binding(1) var voxel_textures: binding_array<texture_3d<u32>>;
@group(1) @binding(2) var<storage, read> gpu_aabbs: array<GpuAabb>;
// %%STATS_BIND%%
@group(1) @binding(4) var acc_struct: acceleration_structure;

@group(2) @binding(0) var out_color: texture_storage_2d<rgba8unorm, write>;

// --- Chunk hierarchical mip DDA (ported verbatim from chunk.wgsl) ----------
//
// Phase-2 bounded six-frame traversal: root mip 5 (8^3 cells, 4 m cells),
// refine occupied parents as 2^3 child frames down to mip 0. The only
// differences from the fragment version: the DDA is a callable function
// (dda_chunk) keyed by TLAS instance index, and a miss returns HitResult(-1,
// 0) instead of discarding — there is no depth output, no frag_depth, no
// early-Z interplay in the compute path.

const CHUNK_WORLD_SIZE: f32 = 32.0;
const ROOT_MIP: u32 = 5u;
const ROOT_GRID_SIZE: i32 = 8;
const ROOT_CELL_SIZE: f32 = 4.0;

const T_EPS: f32 = 1e-6;
const PARALLEL_EPS: f32 = 1e-8;
const RAY_LENGTH_EPS: f32 = 1e-8;
const INF: f32 = 1.0e30;

const ROOT_CELL_CAP: i32 = 24;
const REFINEMENT_CELL_CAP: i32 = 8;
const GLOBAL_CELL_CAP: i32 = 2048;
const TRAVERSAL_BOUND: i32 = 16384;

struct TraversalFrame {
    mip: u32,
    tex_origin: vec3<i32>,
    interval: vec2<f32>,
    cell: vec3<i32>,
    t: f32,
    t_max: vec3<f32>,
    t_delta: vec3<f32>,
    steps_taken: i32,
};

fn ray_aabb(origin: vec3<f32>, dir: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>) -> vec2<f32> {
    var t_enter: f32 = 0.0;
    var t_exit: f32 = INF;

    for (var i: i32 = 0; i < 3; i++) {
        let o = origin[i];
        let d = dir[i];
        let lo = bmin[i];
        let hi = bmax[i];
        if (abs(d) < PARALLEL_EPS) {
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

fn cell_size_at_mip(mip: u32) -> vec3<f32> {
    return vec3<f32>(CHUNK_WORLD_SIZE / f32(256u >> mip));
}

fn grid_size_at_mip(mip: u32) -> i32 {
    return select(2, ROOT_GRID_SIZE, mip == ROOT_MIP);
}

fn init_frame(
    origin: vec3<f32>,
    dir: vec3<f32>,
    chunk_origin: vec3<f32>,
    mip: u32,
    tex_origin: vec3<i32>,
    interval: vec2<f32>,
) -> TraversalFrame {
    var frame: TraversalFrame;
    frame.mip = mip;
    frame.tex_origin = tex_origin;
    frame.interval = interval;
    frame.steps_taken = 0;

    let grid_size = grid_size_at_mip(mip);
    let cell_size = cell_size_at_mip(mip);
    let bounds_min = chunk_origin + vec3<f32>(tex_origin) * cell_size;
    let bounds_max = bounds_min + cell_size * vec3<f32>(f32(grid_size));
    let entry = origin + dir * interval.x;
    let local = (entry - bounds_min) / cell_size;

    var cell = vec3<i32>(0);
    var t_max = vec3<f32>(INF);
    var t_delta = vec3<f32>(INF);

    for (var i: i32 = 0; i < 3; i = i + 1) {
        var c = i32(floor(local[i]));
        let near_boundary = abs(local[i] - round(local[i])) * cell_size[i] <= T_EPS;
        if (dir[i] < 0.0 && near_boundary) {
            c = c - 1;
        }
        c = clamp(c, 0, grid_size - 1);
        cell[i] = c;

        if (abs(dir[i]) < PARALLEL_EPS) {
            t_max[i] = INF;
            t_delta[i] = INF;
        } else {
            t_delta[i] = cell_size[i] / abs(dir[i]);
            let boundary = select(
                bounds_min[i] + f32(c) * cell_size[i],
                bounds_min[i] + f32(c + 1) * cell_size[i],
                dir[i] > 0.0,
            );
            t_max[i] = (boundary - origin[i]) / dir[i];
        }
    }

    frame.cell = cell;
    frame.t = interval.x;
    frame.t_max = t_max;
    frame.t_delta = t_delta;
    return frame;
}

fn advance_frame(frame: TraversalFrame, dir: vec3<f32>) -> TraversalFrame {
    var f = frame;
    let min_t = min(min(f.t_max.x, f.t_max.y), f.t_max.z);
    f.t = min_t;
    for (var i: i32 = 0; i < 3; i = i + 1) {
        if (f.t_max[i] - min_t <= T_EPS) {
            f.cell[i] = f.cell[i] + select(-1, 1, dir[i] > 0.0);
            f.t_max[i] = f.t_max[i] + f.t_delta[i];
        }
    }
    return f;
}

// Traverses one chunk (TLAS instance `chunk_index`) from the ray's AABB
// entry. On a mip-0 voxel hit writes `(*out)` and returns true; returns false
// on miss. `chunk_index` is both the TLAS instance index and the
// voxel_textures binding-array index.
//
// The result is passed through an out-pointer (not returned) because naga
// rejects member access on a struct returned by a function call inside the
// ray-query loop's conditional blocks (scoping bug); a function-scope var is
// safe.
fn dda_chunk(
    origin: vec3<f32>,
    dir: vec3<f32>,
    chunk_index: u32,
    out: ptr<function, HitResult>,
) -> bool {
    let aabb = gpu_aabbs[chunk_index];
    let bmin = vec3<f32>(aabb.min_x, aabb.min_y, aabb.min_z);
    let bmax = vec3<f32>(aabb.max_x, aabb.max_y, aabb.max_z);

    let span = ray_aabb(origin, dir, bmin, bmax);
    if (span[1] < 0.0) {
        return false;
    }
    // Half-open span: reject point-only edge/corner contact.
    if (span[1] - span[0] <= T_EPS) {
        return false;
    }

    var frames: array<TraversalFrame, 6>;
    var stack_len: i32 = 0;
    var processed_cells: i32 = 0;

    frames[0] = init_frame(origin, dir, bmin, ROOT_MIP, vec3<i32>(0), span);
    stack_len = 1;

    for (var iter: i32 = 0; iter < TRAVERSAL_BOUND; iter = iter + 1) {
        if (stack_len == 0) {
            return false;
        }

        let top_idx = stack_len - 1;
        var top = frames[top_idx];

        let local_cap = select(REFINEMENT_CELL_CAP, ROOT_CELL_CAP, top.mip == ROOT_MIP);
        let interval_empty = (top.interval.y - top.interval.x) <= T_EPS;
        let at_or_after_exit = top.t >= top.interval.y - T_EPS;
        if (top.steps_taken >= local_cap || interval_empty || at_or_after_exit) {
            stack_len = stack_len - 1;
            continue;
        }

        let next_boundary = min(min(top.t_max.x, top.t_max.y), top.t_max.z);
        let cell_exit = min(next_boundary, top.interval.y);
        let width = cell_exit - top.t;

        if (width <= T_EPS) {
            frames[top_idx] = advance_frame(top, dir);
            continue;
        }

        processed_cells = processed_cells + 1;
        // %%STATS_CELLS%%
        if (processed_cells > GLOBAL_CELL_CAP) {
            return false;
        }
        top.steps_taken = top.steps_taken + 1;
        let coord = top.tex_origin + top.cell;
        let mat = textureLoad(voxel_textures[chunk_index], coord, top.mip).x;

        if (top.mip == 0u) {
            if (mat != 0u) {
                // %%STATS_HIT%%
                (*out).t = top.t;
                (*out).mat = mat;
                return true;
            }
            frames[top_idx] = advance_frame(top, dir);
            continue;
        }

        if (mat != 0u) {
            let parent_entry = top.t;
            let parent_next = next_boundary;
            let child_exit = min(parent_next, top.interval.y);
            let child_tex_origin = 2 * (top.tex_origin + top.cell);
            let child_mip = top.mip - 1u;

            frames[top_idx] = advance_frame(top, dir);

            if (child_exit - parent_entry > T_EPS && stack_len < 6) {
                frames[stack_len] = init_frame(
                    origin,
                    dir,
                    bmin,
                    child_mip,
                    child_tex_origin,
                    vec2<f32>(parent_entry, child_exit),
                );
                stack_len = stack_len + 1;
            }
        } else {
            frames[top_idx] = advance_frame(top, dir);
        }
    }

    return false;
}

// Fullscreen ray-query pass (Design A): one ray per pixel against the chunk
// TLAS; the chunk DDA is the procedural intersection test.
@compute @workgroup_size(8, 8)
fn rq_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let target_size = textureDimensions(out_color);
    if (gid.x >= target_size.x || gid.y >= target_size.y) {
        return;
    }
    // %%STATS_PIXEL%%

    let pixel_center = vec2<f32>(gid.xy) + vec2<f32>(0.5);
    let in_uv = pixel_center / vec2<f32>(target_size.xy);
    // Screen row 0 is clip.y = +1 (top); `in_uv` grows downward, so d.y is
    // mirrored. This matches the raster path, where the fragment at the top
    // row sits at the top of the view frustum.
    let d = vec2<f32>(in_uv.x * 2.0 - 1.0, 1.0 - in_uv.y * 2.0);

    let origin = (camera.view_inv * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    let temp = camera.proj_inv * vec4<f32>(d.x, d.y, 1.0, 1.0);
    let dir = normalize((camera.view_inv * vec4<f32>(normalize(temp.xyz), 0.0)).xyz);

    var color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    var res: HitResult;
    var found = false;

    var rq: ray_query;
    rayQueryInitialize(&rq, acc_struct, RayDesc(0u, 0xFFu, 0.001, 10000.0, origin, dir));
    while (rayQueryProceed(&rq)) {
        let c = rayQueryGetCandidateIntersection(&rq);
        if (c.kind == RAY_QUERY_INTERSECTION_AABB) {
            if (dda_chunk(origin, dir, c.primitive_index, &res)) {
                // naga 24 rejects a computed expression as the generate
                // distance inside the loop (NotInScope); binding the value to
                // a `let` first avoids the bug.
                let hit_t = res.t;
                found = true;
                rayQueryGenerateIntersection(&rq, hit_t);
            }
        }
    }

    let committed = rayQueryGetCommittedIntersection(&rq);
    if (committed.kind == RAY_QUERY_INTERSECTION_GENERATED && found) {
        color = palette[res.mat];
    }

    textureStore(out_color, gid.xy, color);
}

// --- Blit: fullscreen quad sampling the storage texture to the surface ------
// Verbatim from the upstream `ray_aabb_compute` example (screenshot-tested
// orientation): storage row 0 maps to the top of the screen.

struct BlitOut {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
};

@vertex
fn blit_vs_main(@builtin(vertex_index) vertex_index: u32) -> BlitOut {
    var result: BlitOut;
    let x = i32(vertex_index) / 2;
    let y = i32(vertex_index) & 1;
    let tc = vec2<f32>(
        f32(x) * 2.0,
        f32(y) * 2.0
    );
    result.position = vec4<f32>(
        tc.x * 2.0 - 1.0,
        1.0 - tc.y * 2.0,
        0.0, 1.0
    );
    result.tex_coords = tc;
    return result;
}

@group(0) @binding(0) var blit_color: texture_2d<f32>;
@group(0) @binding(1) var blit_sampler: sampler;

@fragment
fn blit_fs_main(vertex: BlitOut) -> @location(0) vec4<f32> {
    return textureSample(blit_color, blit_sampler, vertex.tex_coords);
}
