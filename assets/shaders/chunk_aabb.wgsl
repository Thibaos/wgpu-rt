enable wgpu_binding_array;

const INF: f32 = 1e30;
const NEG_INF: f32 = -1e30;
const MAX_STEPS: i32 = 768;
const CHUNK_SIDE: f32 = 255.0;
const VOXEL_SCALE: f32 = 0.125;
const CHUNK_WORLD: f32 = CHUNK_SIDE * VOXEL_SCALE;
const ENTRY_EPSILON: f32 = 1e-4;

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
};

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(1) @binding(0) var<storage, read> palette: array<vec4<f32>>;
@group(1) @binding(1) var voxel_textures: binding_array<texture_3d<u32>>;

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
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
    var out: VertexOutput;
    out.position = camera.view_proj * model_matrix * model.position;
    out.tex_coord = model.tex_coord;
    out.chunk_id = instance_index;
    out.chunk_origin = instance.chunk_origin_in.xyz;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    let out_color = vec4<f32>(in.tex_coord.x, in.tex_coord.y, 0.0, 1.0);
    return FragmentOutput(out_color, 0.0);
}
