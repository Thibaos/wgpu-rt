const brick_size: i32 = 8;
const cell_members: i32 = brick_size * brick_size * brick_size / 32;

const EPSILON: f32 = 0.0001;
const FAR: f32 = 1000000.;

struct Uniforms {
    view_inv: mat4x4<f32>,
    proj_inv: mat4x4<f32>,
    palette: array<vec4<f32>, 256>,
};

struct Brick {
    data: array<u32, cell_members>
}

struct Brickmap {
    data: array<u32>,
}

struct Intersection {
    distance: f32,
    normal: vec3<f32>
}

@group(0) @binding(0)
var output: texture_storage_2d<rgba8unorm, write>;

@group(0) @binding(1)
var<uniform> uniforms: Uniforms;

@group(0) @binding(2)
var<storage> brickmap: Brickmap;

fn intersect_brick(origin: vec3<f32>, direction: vec3<f32>, brick: Brick) -> Intersection {
    var pos = vec3<i32>(origin);

    var cb = vec3<f32>(pos);
    if direction.x > 0.0 {
        cb.x = f32(pos.x + 1);
    }
    if direction.y > 0.0 {
        cb.x = f32(pos.y + 1);
    }
    if direction.z > 0.0 {
        cb.x = f32(pos.z + 1);
    }

    var out = vec3<i32>(-1);
    if direction.x > 0.0 {
        out.x = brick_size;
    }
    if direction.y > 0.0 {
        out.y = brick_size;
    }
    if direction.z > 0.0 {
        out.z = brick_size;
    }

    let step: vec3<f32> = sign(direction);
    let rdinv = 1.0 / direction;

    var tmax = vec3<f32>(FAR);
    if abs(direction.x) > EPSILON {
        tmax.x = (cb.x - origin.x) * rdinv.x;
    }
    if abs(direction.y) > EPSILON {
        tmax.y = (cb.y - origin.y) * rdinv.y;
    }
    if abs(direction.z) > EPSILON {
        tmax.z = (cb.z - origin.z) * rdinv.z;
    }

    let tdelta = step * rdinv;

    pos = pos % 8;
    var distance: f32 = -1.0;
    var normal = vec3<f32>();

    var step_axis = -1;
    var mask = vec3<f32>();

    while true {
        let sub_data = (pos.x + pos.y * brick_size + pos.z * brick_size * brick_size) / 32;
        let bit = (pos.x + pos.y * brick_size + pos.z * brick_size * brick_size) % 32;

        if (brick.data[sub_data] & (1u << u32(bit))) == 1u {
            if step_axis > -1 {
                normal = vec3<f32>();
                normal[step_axis] = -step[step_axis];
                distance = tmax[step_axis] - tdelta[step_axis];
            }

            return Intersection(distance, normal);
        }

        if tmax.x < tmax.y {
            if tmax.x < tmax.z {
                step_axis = 0;
            } else {
                step_axis = 2;
            }
        } else {
            if tmax.y < tmax.z {
                step_axis = 1;
            } else {
                step_axis = 2;
            }
        }
        mask.x = f32(tmax.x < tmax.y && tmax.x < tmax.z);
        mask.y = f32(tmax.y <= tmax.x && tmax.y < tmax.z);
        mask.z = f32(tmax.z <= tmax.x && tmax.z <= tmax.y);

        pos += vec3<i32>(mask * step);
        if pos[step_axis] == out[step_axis] {
            break;
        }
        tmax += mask * tdelta;
    }

    return Intersection(distance, normal);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let target_size = textureDimensions(output);
    var color = vec4<f32>(vec2<f32>(global_id.xy) / vec2<f32>(target_size), 0.0, 1.0);

	let pixel_center = vec2<f32>(global_id.xy) + vec2<f32>(0.5);
	let in_uv = pixel_center/vec2<f32>(target_size.xy);
	let d = in_uv * 2.0 - 1.0;

	let origin = (uniforms.view_inv * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
	let temp = uniforms.proj_inv * vec4<f32>(d.x, d.y, 1.0, 1.0);
	let direction = (uniforms.view_inv * vec4<f32>(normalize(temp.xyz), 0.0)).xyz;

	if brickmap.data[0] == 0 {
	    color += vec4<f32>(0.0);
	}

    textureStore(output, global_id.xy, color);
}
