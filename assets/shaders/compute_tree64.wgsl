const brick_size: i32 = 8;

const grid_size: i32 = 512;
const grid_height: i32 = 512;

const supergrid_cell_size: i32 = 16;
const supergrid_xy: i32 = grid_size / brick_size / supergrid_cell_size;
const supergrid_z: i32 = grid_height / brick_size / supergrid_cell_size;
const flat_supergrid_size: i32 = supergrid_xy * supergrid_xy * supergrid_z;

const cells: i32 = grid_size / brick_size;
const cells_height: i32 = grid_height / brick_size;
// The amount of uint32_t members holding voxel bit data
const cell_members: i32 = brick_size * brick_size * brick_size / 32;

const EPSILON: f32 = 0.0001;
const FAR: f32 = 1000000.;

// LoD distance for blocksize 1x1x1 representing 8x8x8
const lod_distance_8x8x8: i32 = 600000;
// LoD distance for blocksize 2x2x2 representing 8x8x8
const lod_distance_2x2x2: i32 = 100000;

const brick_index_bits: u32 = 0xFFFu;
const brick_lod_bits: u32 = 0xFF000u;
const brick_loaded_bit: u32 = 0x80000000u;
const brick_unloaded_bit: u32 = 0x40000000u;
const brick_requested_bit: u32 = 0x20000000u;

const brick_load_queue_size: u32 = 1024;

struct Uniforms {
    view_inv: mat4x4<f32>,
    proj_inv: mat4x4<f32>,
    palette: array<vec4<f32>, 256>,
};

struct Brick {
    data: array<u32, cell_members>
}

struct Brickmap {
    indices: array<u32, flat_supergrid_size>,
    brick_load_queue: array<vec3<i32>, flat_supergrid_size>,
    brick_load_queue_count: u32,
    bricks: array<Brick, flat_supergrid_size>,
}

struct Ray {
    origin: vec3<f32>,
    direction: vec3<f32>,
}

struct Camera {
    right: vec3<f32>,
    up: vec3<f32>,
    direction: vec3<f32>,
    origin: vec3<f32>
}

@group(0) @binding(0)
var output: texture_storage_2d<rgba8unorm, write>;

@group(0) @binding(1)
var<uniform> uniforms: Uniforms;

@group(0) @binding(2)
var<storage> brickmap: Brickmap;

fn intersect_aabb_branchless2(origin: vec3<f32>, direction: vec3<f32>, tmin: ptr<function, f32>) -> bool {
	var box_min = vec3<f32>(0);
	var box_max = vec3<f32>(f32(grid_size), f32(grid_size), f32(grid_height));

	let t1 = (box_min - origin) / direction;
	let t2 = (box_max - origin) / direction;
	let tMin = min(t1, t2);
	let tMax = max(t1, t2);

	*tmin = max(max(tMin.x, 0.0), max(tMin.y, tMin.z));
	return min(tMax.x, min(tMax.y, tMax.z)) > *tmin;
}

fn intersect_byte(origin: vec3<f32>, direction: vec3<f32>, normal: ptr<function, vec3<f32>>, distance: ptr<function, f32>, byte: u32) -> bool {
	var pos = vec3<i32>(floor(origin));

	var cb = vec3<i32>(pos);
	if direction.x > 0.0 {
	    cb.x += 1;
	}
	if direction.y > 0.0 {
	    cb.y += 1;
	}
	if direction.z > 0.0 {
	    cb.z += 1;
	}
	var out = vec3<i32>(-1);
	if direction.x > 0.0 {
	    out.x = 2;
	}
	if direction.y > 0.0 {
	    out.y = 2;
	}
	if direction.z > 0.0 {
	    out.z = 2;
	}

	let step = sign(direction);

	var rdinv = 1.f / direction;
	if abs(direction.x) < EPSILON {
		rdinv.x = 0.0;
	}
	if abs(direction.y) < EPSILON {
		rdinv.y = 0.0;
	}
	if abs(direction.z) < EPSILON {
		rdinv.z = 0.0;
	}

	var tmax = vec3<f32>(FAR);
	if abs(direction.x) > EPSILON {
		tmax.x = (f32(cb.x) - origin.x) * rdinv.x;
	}
	if abs(direction.y) > EPSILON {
		tmax.y = (f32(cb.y) - origin.y) * rdinv.y;
	}
	if abs(direction.z) > EPSILON {
		tmax.z = (f32(cb.z) - origin.z) * rdinv.z;
	}
	let tdelta = step * rdinv;

	pos = pos % 2;

	*distance = 0.0;
	var step_axis = -1;
	var mask = vec3<f32>();
	// Stepping through grid
	while true {
		if ((byte >> 24) & (1u << u32(pos.x + pos.y * 2 + pos.z * 4))) > 0u {
			if step_axis > -1 {
				*normal = vec3<f32>(0.0);
				(*normal)[step_axis] = -step[step_axis];
				*distance = tmax[step_axis] - tdelta[step_axis];
			}
			return true;
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
	return false;
}

fn intersect_brick(origin: vec3<f32>, direction: vec3<f32>, normal: ptr<function, vec3<f32>>, distance: ptr<function, f32>, brick: Brick) -> bool {
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
    *distance = -1.0;
    *normal = vec3<f32>();

    var step_axis = -1;
    var mask = vec3<f32>();

    while true {
        let sub_data = (pos.x + pos.y * brick_size + pos.z * brick_size * brick_size) / 32;
        let bit = (pos.x + pos.y * brick_size + pos.z * brick_size * brick_size) % 32;

        if (brick.data[sub_data] & (1u << u32(bit))) > 0u {
            if step_axis > -1 {
                *normal = vec3<f32>();
                (*normal)[step_axis] = -step[step_axis];
                *distance = tmax[step_axis] - tdelta[step_axis];
            }

            return true;
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

    return false;
}


fn intersect_voxel(origin: ptr<function, vec3<f32>>, direction: vec3<f32>, camera_position: vec3<i32>, scene: ptr<function, Brickmap>, normal: ptr<function, vec3<f32>>, distance: ptr<function, f32>) -> bool {
    var tminn: f32;
	if !intersect_aabb_branchless2(*origin, direction, &tminn) {
		return false;
	}

	// Move ray to hitpoint
	if tminn > 0.0 {
		*origin += direction * tminn;

		let scale = vec3<f32>(1.0 / (f32(grid_size) / f32(grid_height)), 1.0 / (f32(grid_size) / f32(grid_height)), 1.0 / (f32(grid_height) / f32(grid_height)));
		let grid_center = vec3<f32>(f32(grid_size) / 2.0, f32(grid_size) / 2.0, f32(grid_height) / 2.0);

		var to_center = abs(grid_center - *origin) * scale;
		let signs = sign(*origin - grid_center);

		to_center /= max(to_center.x, max(to_center.y, to_center.z));
		*normal = signs * trunc(to_center + EPSILON);

		*origin -= *normal * EPSILON;
	}

	*origin /= 8.0;
	var pos: vec3<i32> = vec3<i32>(*origin);

	// Needed because sometimes the AABB intersect returns true while the ray is actually outside slightly. Only happens for faces that touch the AABB sides
	if pos.x < 0 || pos.x >= cells || pos.y < 0 || pos.y >= cells || pos.z < 0 || pos.z >= cells_height {
		return false;
	}

	var cb = vec3<i32>(pos);
	if direction.x > 0.0 {
	    cb.x += 1;
	}
	if direction.y > 0.0 {
	    cb.y += 1;
	}
	if direction.z > 0.0 {
	    cb.z += 1;
	}

	var out = vec3<i32>(-1);
	if direction.x > 0.0 {
	    out.x = cells;
	}
	if direction.y > 0.0 {
	    out.y = cells;
	}
	if direction.z > 0.0 {
	    out.z = cells_height;
	}

	let step: vec3<f32> = sign(direction);

	// Produces INFINITY when a direction is zero
	var rdinv: vec3<f32> = 1.0 / direction;
	if abs(direction.x) < EPSILON {
		rdinv.x = 0.0;
	}
	if abs(direction.y) < EPSILON {
		rdinv.y = 0.0;
	}
	if abs(direction.z) < EPSILON {
		rdinv.z = 0.0;
	}

	var tmax = vec3<f32>(FAR);
	if abs(direction.x) > EPSILON {
		tmax.x = (f32(cb.x) - origin.x) * rdinv.x;
	}
	if abs(direction.y) > EPSILON {
		tmax.y = (f32(cb.y) - origin.y) * rdinv.y;
	}
	if abs(direction.z) > EPSILON {
		tmax.z = (f32(cb.z) - origin.z) * rdinv.z;
	}

	let tdelta: vec3<f32> = step * rdinv;

	var step_axis = -1;
	var mask: vec3<f32>;

	while true {
		//unsigned int supercell_index = morton(pos.x / supergrid_cell_size) + (morton(pos.y / supergrid_cell_size) << 1) + (morton(pos.z / supergrid_cell_size) << 2);
		//int supercell_index = (pos.x >> 4) + (pos.y >> 4) * supergrid_xy + (pos.z >> 4) * supergrid_xy * supergrid_xy;
		//uint32_t& index = scene.indices[supercell_index][(pos.x & 15) + (pos.y & 15) * supergrid_cell_size + (pos.z & 15) * supergrid_cell_size * supergrid_cell_size];

		let supercell_index = pos.x / supergrid_cell_size + (pos.y / supergrid_cell_size) * supergrid_xy + (pos.z / supergrid_cell_size) * supergrid_xy * supergrid_xy;
		// let index = scene.indices[supercell_index][(pos.x % supergrid_cell_size) + (pos.y % supergrid_cell_size) * supergrid_cell_size + (pos.z % supergrid_cell_size) * supergrid_cell_size * supergrid_cell_size];

		let index = 0u;

		if index > 0u {
			var new_distance = 0.0;
			if step_axis != -1 {
				*normal = vec3<f32>(0.0);
				(*normal)[step_axis] = -step[step_axis];
				new_distance = tmax[step_axis] - tdelta[step_axis];
			}

			let difference = camera_position - pos;
			let lod_distance_squared = difference.x * difference.x + difference.y * difference.y + difference.z * difference.z;
			var sub_distance = 0.0;

			if lod_distance_squared > lod_distance_8x8x8 {
				*distance = new_distance * 8.0 + tminn;
				return true;
			} else if lod_distance_squared > lod_distance_2x2x2 {
				// For some reason the normal displacement has to be made even smaller
				if intersect_byte((*origin + direction * new_distance) * 2.0 - *normal * 0.2 * EPSILON, direction, normal, &sub_distance, (index & brick_lod_bits) >> 12) {
					*distance = new_distance * 8.0 + sub_distance * 4.0 + tminn;
					return true;
				}
			} else {
				if (index & brick_loaded_bit) > 0u {
					// var p = scene.bricks[supercell_index];
					// let p_i = supercell_index * (index & brick_index_bits);
					// if intersect_brick((origin + direction * new_distance) * 8.0 - normal * EPSILON, direction, normal, sub_distance, &p[p_i]) {
					// 	*distance = new_distance * 8.0 + sub_distance + tminn;
					// 	return true;
					// }
				} else if (index & brick_unloaded_bit) > 0u {
				    let old = index | brick_requested_bit;
					// let old = atomicOr(index, brick_requested_bit);

					if (old & brick_requested_bit) == 0u {
						// request chunk to be loaded

						let load_index = scene.brick_load_queue_count + 1;
						// let load_index = atomicAdd(scene.brick_load_queue_count, 1);
						if load_index < brick_load_queue_size {
							(*scene).brick_load_queue[load_index] = pos;
						} else {
							// atomicAnd(&index, ~brick_requested_bit);
							// ToDo happens a lot. Fix?
						}
					}

					*distance = new_distance * 8.0 + tminn;

					return true;
				}
			}
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

	return false;
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

	var tmin = -1.0;

	let b = brickmap.indices[0];

	if intersect_aabb_branchless2(origin, direction, &tmin) {
        color = vec4<f32>(tmin / f32(grid_size), tmin / f32(grid_size), tmin / f32(grid_size), 1.0);
	}

    textureStore(output, global_id.xy, color);
}
