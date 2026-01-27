use wgpu_rt::ray_cube_compute;

fn main() {
    wgpu_rt::framework::run::<ray_cube_compute::App>("ray-cube");
}
