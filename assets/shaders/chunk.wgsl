enable wgpu_binding_array;

// %%STATS_DECLS%%

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

// Writes the real per-voxel hit depth via @builtin(frag_depth). Note: a
// frag_depth write forces the depth test to run LATE (after the fragment
// shader), so hardware early-Z never engages on this pass — that trade-off
// is accepted so the depth buffer holds true surface depths (needed by any
// downstream depth consumer).
struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

// --- Chunk hierarchical mip DDA (phase 2: bounded six-frame traversal) -----
//
// The chunk is a 256^3 volume; 1 voxel = VOXEL_SCALE (0.125 m), so a chunk
// spans 32 m. The 3D texture has 9 mip levels (256 -> 128 -> ... -> 1).
//
// Phase 2 starts at mip 5 (8^3 cells, 4 m cells), descends 5 -> 4 -> 3 -> 2 ->
// 1 -> 0, and uses mips 5 through 1 for occupancy only. Mip 0 supplies
// material, entry depth, and the hit voxel. Each refinement is a local 2^3
// DDA bounded by the occupied parent cell, with
// child_tex_origin = 2 * (parent.tex_origin + parent.cell). Traversal is
// half-open: intervals are [t_enter, t_exit). A non-zero coarse value is only
// a hint; it is never a color or a hit.
//
// CHUNK_WORLD_SIZE / ROOT_GRID_SIZE / ROOT_CELL_SIZE are hardcoded here and
// MUST match the CPU source of truth: CHUNK_TEXTURE_SIZE.width * VOXEL_SCALE
// (32.0).
const CHUNK_WORLD_SIZE: f32 = 32.0;
const ROOT_MIP: u32 = 5u;       // 256 >> 5 = 8 cells per axis
const ROOT_GRID_SIZE: i32 = 8;  // 256 >> ROOT_MIP
const ROOT_CELL_SIZE: f32 = 4.0; // CHUNK_WORLD_SIZE / ROOT_GRID_SIZE (metres)

const T_EPS: f32 = 1e-6;        // metre comparison epsilon (comparisons only)
const PARALLEL_EPS: f32 = 1e-8; // below this |dir component| the ray is parallel to the slab
const RAY_LENGTH_EPS: f32 = 1e-8; // camera-to-fragment ray length guard (metres)
const INF: f32 = 1.0e30;

const ROOT_CELL_CAP: i32 = 24;      // samples per root frame (diagonal of 8^3 is ~14, headroom)
const REFINEMENT_CELL_CAP: i32 = 8; // samples per child frame (2^3 grid: max 4 cells on a ray)
const GLOBAL_CELL_CAP: i32 = 2048;  // total positive-width samples per fragment
// Static outer traversal-loop bound. Derivation: each iteration either
// samples one positive-width cell (global cap 2048 samples; the 2049th
// discards), pops a frame, or skips a zero-width cell. Pops are bounded by
// pushes + 1, and pushes accompany mip-1..5 samples (<= total samples), so
// pops <= 2049; a zero-width skip is immediately followed by a sample or a
// pop, so skips <= pops + samples + 1 = 4098. Total iterations <= 2048 + 2049
// + 4098 = 8195; 16384 leaves margin. A naive GLOBAL_CELL_CAP + stack_depth
// bound is NOT sufficient — pops and skips are bounded by the number of
// samples, not by the six-frame stack depth. No WGSL `loop` or other
// unbounded control-flow construct.
const TRAVERSAL_BOUND: i32 = 16384;

// One stack frame for the bounded hierarchical traversal. Invariants:
//   - tex_origin + cell is the texture coordinate of the current cell at mip;
//   - t is that cell's raw ray-entry parameter;
//   - interval.y is the exclusive region exit (half-open [interval.x, interval.y)).
struct TraversalFrame {
    mip: u32,
    grid_size: i32,
    tex_origin: vec3<i32>,
    bounds_min: vec3<f32>,
    bounds_max: vec3<f32>,
    interval: vec2<f32>,
    cell: vec3<i32>,
    t: f32,
    t_max: vec3<f32>,
    t_delta: vec3<f32>,
    axis_step: vec3<i32>,
    steps_taken: i32,
};

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

// Builds a traversal frame whose first sample uses `interval.x` (the interval
// entry parameter), never min(t_max) (which is the cell exit).
//
// Applies direction-aware negative-boundary correction before floor: when the
// ray component is negative and the normalized coordinate is within T_EPS of
// an integer boundary, the floored cell is decremented on that axis before
// clamping. Positive-direction boundary entries select the cell on the
// positive side. Every axis is clamped to [0, grid_size - 1].
//
// t_delta and t_max are per-axis and derived from the frame's own bounds:
// cell_size = (bounds_max - bounds_min) / grid_size, NOT
// CHUNK_WORLD_SIZE / grid_size (wrong for child frames: a mip-4 child of a
// root cell is 2 m, not 16 m). t_max is an absolute ray parameter to the next
// boundary on that axis; the raw/unmodified value is preserved.
fn init_frame(
    origin: vec3<f32>,
    dir: vec3<f32>,
    mip: u32,
    grid_size: i32,
    tex_origin: vec3<i32>,
    bounds_min: vec3<f32>,
    bounds_max: vec3<f32>,
    interval: vec2<f32>,
) -> TraversalFrame {
    var frame: TraversalFrame;
    frame.mip = mip;
    frame.grid_size = grid_size;
    frame.tex_origin = tex_origin;
    frame.bounds_min = bounds_min;
    frame.bounds_max = bounds_max;
    frame.interval = interval;
    frame.steps_taken = 0;

    let cell_size = (bounds_max - bounds_min) / vec3<f32>(f32(grid_size));
    let entry = origin + dir * interval.x;
    let local = (entry - bounds_min) / cell_size;

    var cell = vec3<i32>(0);
    var t_max = vec3<f32>(INF);
    var t_delta = vec3<f32>(INF);
    var axis_step = vec3<i32>(0);

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
            axis_step[i] = 0;
        } else {
            axis_step[i] = select(-1, 1, dir[i] > 0.0);
            t_delta[i] = cell_size[i] / abs(dir[i]);
            // Absolute ray parameter of the next boundary on this axis.
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
    frame.axis_step = axis_step;
    return frame;
}

// Advances the frame to the raw minimum of the three t_max values, then
// advances EVERY axis whose boundary is within T_EPS of that minimum
// (all-axis tie behavior for edge/corner ties). Parallel axes (step 0,
// t_max = INF) are never advanced by this rule.
fn advance_frame(frame: TraversalFrame) -> TraversalFrame {
    var f = frame;
    let min_t = min(min(f.t_max.x, f.t_max.y), f.t_max.z);
    f.t = min_t;
    for (var i: i32 = 0; i < 3; i = i + 1) {
        if (f.t_max[i] - min_t <= T_EPS) {
            f.cell[i] = f.cell[i] + f.axis_step[i];
            f.t_max[i] = f.t_max[i] + f.t_delta[i];
        }
    }
    return f;
}

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    // %%STATS_FRAGMENT%%
    let chunk_origin = in.chunk_origin;
    let bmin = chunk_origin;
    let bmax = chunk_origin + vec3<f32>(CHUNK_WORLD_SIZE);

    let origin = camera.camera_pos.xyz;
    let delta = in.world_position - origin;
    // Guard the camera-to-fragment vector against zero length before
    // normalize; a zero-length ray must miss, not produce NaN.
    if (dot(delta, delta) <= RAY_LENGTH_EPS * RAY_LENGTH_EPS) {
        discard;
    }
    let dir = normalize(delta);

    let span = ray_aabb(origin, dir, bmin, bmax);
    if (span[1] < 0.0) {
        discard;
    }
    // Half-open span: reject point-only edge/corner contact.
    if (span[1] - span[0] <= T_EPS) {
        discard;
    }

    var frames: array<TraversalFrame, 6>;
    var stack_len: i32 = 0;
    var processed_cells: i32 = 0;

    // Root frame: mip 5, grid 8, tex_origin (0,0,0), chunk AABB, analytic span.
    frames[0] = init_frame(origin, dir, ROOT_MIP, ROOT_GRID_SIZE, vec3<i32>(0), bmin, bmax, span);
    stack_len = 1;

    for (var iter: i32 = 0; iter < TRAVERSAL_BOUND; iter = iter + 1) {
        if (stack_len == 0) {
            discard;
        }

        let top_idx = stack_len - 1;
        var top = frames[top_idx];

        // Pop: the top frame reached its local cap, its interval has no
        // positive width, or its current entry is at/after the exclusive
        // interval exit within T_EPS. Popping a root frame ends the traversal;
        // popping a child resumes the already-advanced parent.
        let local_cap = select(REFINEMENT_CELL_CAP, ROOT_CELL_CAP, top.mip == ROOT_MIP);
        let interval_empty = (top.interval.y - top.interval.x) <= T_EPS;
        let at_or_after_exit = top.t >= top.interval.y - T_EPS;
        if (top.steps_taken >= local_cap || interval_empty || at_or_after_exit) {
            stack_len = stack_len - 1;
            continue;
        }

        // Current cell's raw exit: nearest boundary vs the exclusive exit.
        let next_boundary = min(min(top.t_max.x, top.t_max.y), top.t_max.z);
        let cell_exit = min(next_boundary, top.interval.y);
        let width = cell_exit - top.t;

        if (width <= T_EPS) {
            // Zero-width interval: do not sample it; advance (the loop top
            // pops when the advance reaches the exclusive exit). Zero-width
            // intervals consume neither the per-frame nor the global budget.
            frames[top_idx] = advance_frame(top);
            continue;
        }

        // Positive-width sample.
        processed_cells = processed_cells + 1;
        // %%STATS_CELLS%%
        if (processed_cells > GLOBAL_CELL_CAP) {
            discard;
        }
        top.steps_taken = top.steps_taken + 1;
        let coord = top.tex_origin + top.cell;
        let mat = textureLoad(voxel_textures[in.chunk_id], coord, top.mip).x;

        if (top.mip == 0u) {
            // Mip 0 supplies material, entry depth, and the hit voxel. A zero
            // value here advances exactly as at mip 1..5 — it is not a miss,
            // a discard, or a coarse hit.
            if (mat != 0u) {
                let hit_world = origin + dir * top.t;
                let clip = camera.view_proj * vec4<f32>(hit_world, 1.0);
                // %%STATS_HIT%%
                return FragmentOutput(palette[mat], clip.z / clip.w);
            }
            frames[top_idx] = advance_frame(top);
            continue;
        }

        if (mat != 0u) {
            // Coarse occupancy only: capture the parent cell before advancing,
            // advance the parent (front-to-back sibling order), then push the
            // bounded 2^3 child with the exact texture-origin mapping
            // child_tex_origin = 2 * (parent.tex_origin + parent.cell).
            let parent_entry = top.t;
            let parent_next = next_boundary;
            let child_exit = min(parent_next, top.interval.y);
            let cell_size = (top.bounds_max - top.bounds_min) / vec3<f32>(f32(top.grid_size));
            let cell_bmin = top.bounds_min + vec3<f32>(top.cell) * cell_size;
            let cell_bmax = cell_bmin + cell_size;
            let child_tex_origin = 2 * (top.tex_origin + top.cell);
            let child_mip = top.mip - 1u;

            // Advance the parent before pushing the child so a child miss or
            // cap exhaustion resumes the saved parent state.
            frames[top_idx] = advance_frame(top);

            // A full stack treats the branch as unresolvable and continues
            // the already-advanced parent rather than writing a coarse hit.
            if (child_exit - parent_entry > T_EPS && stack_len < 6) {
                frames[stack_len] = init_frame(
                    origin,
                    dir,
                    child_mip,
                    2,
                    child_tex_origin,
                    cell_bmin,
                    cell_bmax,
                    vec2<f32>(parent_entry, child_exit),
                );
                stack_len = stack_len + 1;
            }
        } else {
            frames[top_idx] = advance_frame(top);
        }
    }

    // No mip-0 hit (root popped, or TRAVERSAL_BOUND exhausted with no hit and
    // no discard taken). A WGSL fragment must terminate on every path, and
    // phase-1's miss == discard contract is preserved.
    discard;
}
