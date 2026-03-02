const PI: f32 = 3.141592;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
};

// meant to be called with 3 vertex indices: 0, 1, 2
// draws one large triangle over the clip space like this:
// (the asterisks represent the clip space bounds)
//-1,1           1,1
// ---------------------------------
// |              *              .
// |              *           .
// |              *        .
// |              *      .
// |              *    .
// |              * .
// |***************
// |            . 1,-1
// |          .
// |       .
// |     .
// |   .
// |.
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var result: VertexOutput;
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

@group(0) @binding(0) var r_color: texture_2d<u32>;
@group(0) @binding(1) var r_sampler: sampler;
@group(1) @binding(0) var<uniform> camera_pos: vec3<f32>;
@group(1) @binding(1) var<uniform> view_inv: mat4x4<f32>;
@group(1) @binding(2) var<uniform> proj_inv: mat4x4<f32>;

@fragment
fn fs_main(vertex: VertexOutput) -> @location(0) vec4<f32> {
    let x = vertex.tex_coords.x;
    let y = vertex.tex_coords.y;

    let px_nds = vertex.tex_coords * 2.0 - 1.0;
    let point_nds = vec3<f32>(px_nds, -1.0);
    let point_ndsh = vec4<f32>(point_nds, 1.0);

    var dir_eye = proj_inv * point_ndsh;
    dir_eye.w = 0.0;

    let dir_world = (view_inv * dir_eye).xyz;

    let ray_direction = normalize(dir_world);
    // var ray_direction = normalize(vec3<f32>(px, py, -1.0) - camera_pos);
    // let render_color = textureSample(r_color, r_sampler, vertex.tex_coords);
    let render_color = textureLoad(r_color, vec2<u32>(vertex.tex_coords * vec2<f32>(1920.0, 1080.0)), 0);
    let r = render_color.r;

    if r > 0 {
        return vec4<f32>(1.0);
    }

    return vec4<f32>(ray_direction, 1.0);
}
