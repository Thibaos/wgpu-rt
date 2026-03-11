// Credits: https://pastebin.com/4LsHNxqZ

struct VertexOutput {
    @location(0) tex_coord: vec2<f32>,
    @builtin(position) position: vec4<f32>,
};

// AABB representing a spatial region covered by an octree node.
// Bitmap encodes the occupancy of the child octants of that node.
struct AABB {
    // world-space min corner of the AABB
    aabb_min: vec3<f32>,
    // the scale of the node represented by the AABB
    // octants internally are half this scale value
    // max corner is `min + vec3<f32>(scale)`
    scale: f32,
    // 2x2x2 occupancy bitmap of the child nodes of an octree (only 8 of 32 bits used)
    // NOTE 4x4x4 fits perfectly in vec2<u32> and is likely better performance overall
    octants: u32,
}

// @group(0) @binding(0) var<storage, read> node_aabbs: array<AABB>;
// @group(1) @binding(0) var<uniform> camera_pos: vec3<f32>;
@group(0) @binding(1) var<uniform> transform: mat4x4<f32>;

// cube face vertex LUT for triangle strip topology
// the 6 faces of a cube have 4 strip vertices each
// corner bitwise encoding: bit0=x, bit1=y, bit2=z
const FACE_STRIP: array<u32, 24u> = array<u32, 24u>(
    0u, 4u, 2u, 6u, // Face 0: -X
    1u, 3u, 5u, 7u, // Face 1: +X
    0u, 1u, 4u, 5u, // Face 2: -Y
    2u, 6u, 3u, 7u, // Face 3: +Y
    0u, 2u, 1u, 3u, // Face 4: -Z
    4u, 5u, 6u, 7u, // Face 5: +Z
);

/// Vertex shader to trigger fragments over regions containing voxels.
/// Submits the 3 front-faces of an AABB in 12 vertices.
/// AABB is the tightened bounding region around a voxel occupancy bitmap.
/// This bitmap will be raymarched by fragments using DDA.
///
/// This shader does manual vertex pulling: a vertex input buffer isn't bound.
/// Shader is dispatched over the `node_aabbs` binding via indirect draw args.
///
/// DrawArgs.instance_count incremented with += 3 for each AABB, one per face.
/// DrawArgs.vertex_count is locked at 4 because faces always have 4 vertices.
/// Pipeline uses TriangleStrip primitive topology so faces can be 4 vertices.

@vertex
fn vertex(
    @location(0) position: vec4<f32>,
    @location(1) tex_coord: vec2<f32>,
    // @builtin(vertex_index) v_index: u32,
    // @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var result: VertexOutput;
    result.tex_coord = tex_coord;
    result.position = transform * position;
    return result;

    // let node_index = instance_index / 3u;
    // let face_local = instance_index % 3u;

    // NOTE this buffer indexing is the only non-uniform memory read
    // latency can be improved by using a vertex input buffer isntead
    // let node = node_aabbs[node_index];
    // let octants = node.octants;
    // for 2x2x2, octants are half size of the node itself
    // since these are octree nodes from any depth, they are of variable size
    // for 4x4x4 or other larger sizes, 0.5 should be adjusted proportionally
    // let octant_scale = node.scale * 0.5;

    // Compute tight AABB from occupied octants in octants bitmask
    // Octant layout: bit = x + 2*y + 4*z, so bits 0-3 have x=0/1 for low z/y,
    // X: bits 1,3,5,7 have x=1; bits 0,2,4,6 have x=0
    // Y: bits 2,3,6,7 have y=1; bits 0,1,4,5 have y=0
    // Z: bits 4,5,6,7 have z=1; bits 0,1,2,3 have z=0
    // let sv = vec3<u32>(octants);
    // let has_lo = vec3<f32>(min(sv & vec3<u32>(0x55u, 0x33u, 0x0Fu), vec3<u32>(1u)));
    // let has_hi = vec3<f32>(min(sv & vec3<u32>(0xAAu, 0xCCu, 0xF0u), vec3<u32>(1u)));
    // let tight_min = node.aabb_min + octant_scale * (vec3<f32>(1.0) - has_lo);
    // let tight_max = node.aabb_min + octant_scale * (vec3<f32>(1.0) + has_hi);
    // let tight_size = tight_max - tight_min;

    // select camera-facing face for this instance's axis
    // let center = tight_min + tight_size * 0.5;
    // let cam_dir = camera_pos - center;
    // let face_x = select(0u, 1u, cam_dir.x >= 0.0);
    // let face_y = select(2u, 3u, cam_dir.y >= 0.0);
    // let face_z = select(4u, 5u, cam_dir.z >= 0.0);

    // bitwise corner extraction for current vertex
    // let face_index = select(select(face_z, face_y, face_local == 1u), face_x, face_local == 0u);
    // let corner_bit = FACE_STRIP[face_index * 4u + v_index];
    // let corner_pos = vec3<f32>(
    //     f32((corner_bit >> 0u) & 1u),
    //     f32((corner_bit >> 1u) & 1u),
    //     f32((corner_bit >> 2u) & 1u),
    // );

    // output gets consumed by fragment shader
//     var out: Params;
//     out.node_index = node_index;
//     out.aabb_min = node.aabb_min;
//     out.octant_scale = octant_scale;
//     out.octants = octants;
//     let corner = tight_min + corner_pos * tight_size;
//     out.clip_pos = camera_view_proj * vec4<f32>(corner, 1.0);
//     out.world_pos = corner;
//     return out;
}

// The `clip_pos` param is used internally by the GPU's rasterization.
// All other params are passed to the fragment to do ray tracing.
// For parameters that do not vary with vertex, use `@interpolate(flat)`.
// Only varying param is `world_pos` which gets interpolated per fragment for correct entry point.
struct Params {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) @interpolate(flat) node_index: u32,
    @location(1) @interpolate(flat) aabb_min: vec3<f32>,
    @location(2) @interpolate(flat) octant_scale: f32,
    @location(3) @interpolate(flat) octants: u32,
    @location(4) world_pos: vec3<f32>,
}

// Used by DDA to prevent the ray from stepping along perfectly aligned axes.
const VERY_BIG: f32 = 1e30;

// DDA per-axis ray direction min magnitude: eliminates `1.0 / dir` overflow.
const VERY_SMALL: f32 = 1e-6;

/// Fragment shader to do DDA over a tiny 2x2x2 voxel region.
/// Scale is needed because the voxels are children of octree nodes which have hierarchical size.
/// NOTE depth buffer is based on vertex geometry and fragment discard, so it lacks per-voxel detail.
/// For per-voxel depth accuracy, make fragments write to an additional 32-bit float render target.
/// Do not do manual depth writing as it disables significant hardware optimizations under the hood.

@fragment
fn fragment(in: VertexOutput) -> @location(0) u32 {
    return 1;
    // return vec4<f32>(1.0);

    // let pos = camera_pos;
    // let dir = normalize(in.world_pos - pos);

    // DDA param setup
    // let inv_dir = 1.0 / dir;
    // moving is a bool vector for branchless axis selection
    // let moving = abs(dir) > vec3<f32>(VERY_SMALL);
    // let dda_step = select(
    //     vec3<i32>(0),
    //     vec3<i32>(sign(dir)),
    //     moving,
    // );

    // starting voxel from rasterized surface point
    // clamped to guard against surface points landing just outside [0, 1]
    // let local = (in.world_pos - in.aabb_min) / in.octant_scale;
    // var voxel = clamp(
    //     vec3<i32>(floor(local)),
    //     vec3<i32>(0),
    //     vec3<i32>(1),
    // );

    // t_delta: t to traverse one voxel in each axis (inf for axis-aligned rays)
    // let t_delta = select(
    //     vec3<f32>(VERY_BIG),
    //     abs(in.octant_scale * inv_dir),
    //     moving,
    // );

    // t_max: t at which we cross the next voxel boundary in each axis
    // let forward_plane = vec3<f32>(voxel + max(dda_step, vec3<i32>(0)));
    // let next_boundary = in.aabb_min + forward_plane * in.octant_scale;
    // var t_max = select(
    //     vec3<f32>(VERY_BIG),
    //     (next_boundary - pos) * inv_dir,
    //     moving,
    // );

    // max DDA iterations for N^3 grid is `3*(N-1)+1`
    // for (var i = 0u; i < 4u; i++) {

        // let octant = u32(voxel.x) + 2u * u32(voxel.y) + 4u * u32(voxel.z);
        // let is_solid = (in.octants & (1u << octant)) != 0u;
        // if is_solid {
            // encodes node_index and octant in R32Uint voxel pointer texture
            // NOTE typically you'd clear the output texture before this pass
            // so texels by default are 0u, which is technically a valid output
            // so the `+1` is used to differentiate between uninitialized and valid
            // when you read back each texel, assuming your texture is R32Uint:
            // `if texel.r == 0u { continue; }`
            // `let node_index = (texel.r - 1u) / 8u;`
            // `let hit_octant = (texel.r - 1u) % 8u;`
            // `/` and `%` are expensive ops, but compiler optimizes this for power of 2s like 8
            // You can replace `/ 8u` with `>> 3u` and `% 8u` with `& 7u` to be sure
            // return in.node_index * 8u + octant + 1u;
        // }

        // branchless axis selection picks smallest t-max
        // let is_min = vec3<bool>(
            // t_max.x <= t_max.y && t_max.x <= t_max.z,
        //     t_max.y  < t_max.x && t_max.y <= t_max.z,
        //     t_max.z  < t_max.x && t_max.z  < t_max.y,
        // );
        // let step_mask = select(
        //     vec3<i32>(0),
        //     vec3<i32>(1),
        //     is_min,
        // );
        // voxel += dda_step * step_mask;
        // if any(voxel < vec3<i32>(0)) || any(voxel > vec3<i32>(1)) {
            // out of bounds: exit the loop
    //         break;
    //     }
    //     t_max += t_delta * select(
    //         vec3<f32>(0.0),
    //         vec3<f32>(1.0),
    //         is_min,
    //     );
    // }

    // discarding means there's a hole and lets the GPU schedule fragments behind this
    // discard;
}
