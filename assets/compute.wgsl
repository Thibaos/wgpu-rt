struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_inverse: mat4x4<f32>,
    proj_inverse: mat4x4<f32>,
};

@group(0)
@binding(0)
var output: texture_storage_2d<bgra8unorm, write>;

@group(1)
@binding(0)
var<uniform> camera: CameraUniform; 

@compute
@workgroup_size(16, 16, 1)
fn compute_ray_tracing(
    @builtin(global_invocation_id)
    gid: vec3<u32>,
) {
    let size = vec2<f32>(textureDimensions(output).xy);

    let pixel_center = vec2<f32>(gid.xy) + vec2<f32>(0.5);
    let in_uv = pixel_center / size;

    let d = in_uv * 2.0 - 1.0;

    let world_origin = (camera.view_inverse * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    let world_target = (camera.proj_inverse * vec4<f32>(d.x, d.y, 1.0, 1.0)).xyz;
    let world_direction = (camera.view_inverse * vec4<f32>(normalize(world_target), 0.0)).xyz;

    let origin = world_origin;
    let direction = world_direction;

    let id = vec2<f32>(f32(gid.x), f32(gid.y));
    let frag_coord = id / size;

    let color = vec4<f32>(f32(frag_coord.x), f32(frag_coord.y), 1.0, 1.0);

    textureStore(output, gid.xy, color);
}
