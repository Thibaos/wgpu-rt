struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_inverse: mat4x4<f32>,
    proj_inverse: mat4x4<f32>,
};

struct Ray {
    origin: vec3<f32>,
    direction: vec3<f32>,
};

struct Sphere {
    center: vec3<f32>,
    radius: f32
};

@group(0)
@binding(0)
var output: texture_storage_2d<bgra8unorm, write>;

@group(1)
@binding(0)
var<uniform> camera: CameraUniform; 

fn ray_hit_sphere(ray: Ray, sphere: Sphere) -> f32 {
    let origin_center = ray.origin - sphere.center;
    let b = dot(origin_center, ray.direction);
    let c = dot(origin_center, origin_center) - sphere.radius * sphere.radius;
    let h = b * b - c;
    if h < 0.0 { return -1.0; }
    let h_sqrt = sqrt(h);
    return min(-b - h_sqrt, -b + h_sqrt);
}

fn render(pixel_pos: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(output).xy);

    let d = pixel_pos * 2.0 - 1.0;

    let world_origin = (camera.view_inverse * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    let world_target = (camera.proj_inverse * vec4<f32>(d.x, d.y, 1.0, 1.0)).xyz;
    let world_direction = (camera.view_inverse * vec4<f32>(normalize(world_target), 0.0)).xyz;

    let ray = Ray(world_origin, world_direction);
    let sphere = Sphere(vec3<f32>(0.0, 0.0, 10.0), 1.0);
    let hit_sphere = ray_hit_sphere(ray, sphere);

    return vec4<f32>(vec3<f32>(clamp(hit_sphere, 0.0, 1.0)), 1.0);
}


@compute
@workgroup_size(16, 16, 1)
fn compute_ray_tracing(
    @builtin(global_invocation_id)
    gid: vec3<u32>,
) {
    let size = vec2<f32>(textureDimensions(output).xy);

    let location = vec2<i32>(i32(gid.x), i32(gid.y));
    let pixel = vec2<f32>(f32(gid.x) / size.x, f32(gid.y) / size.y);
    var color = render(pixel);

    textureStore(output, gid.xy, color);
}
