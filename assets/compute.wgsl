@group(0)
@binding(0)
var output: texture_storage_2d<bgra8unorm, write>;

@compute
@workgroup_size(16, 16, 1)
fn compute_ray_tracing(
    @builtin(global_invocation_id)
    gid: vec3<u32>,
) {
    let texture_size = vec2<f32>(textureDimensions(output).xy);
    let id = vec2<f32>(f32(gid.x), f32(gid.y));
    let frag_coord = id / texture_size;

    let color = vec4<f32>(f32(frag_coord.x), f32(frag_coord.y), 1.0, 1.0);

    textureStore(output, gid.xy, color);
}
