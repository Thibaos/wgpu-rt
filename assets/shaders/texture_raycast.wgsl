@binding(0) @group(0) var output : texture_storage_2d<rgba8unorm, write>;

struct _MatrixStorage_float4x4_ColMajorstd140_0
{
    @align(16) data_0 : array<vec4<f32>, i32(4)>,
};

struct Camera
{
    @align(16) pos_0 : vec3<f32>,
    @align(16) viewInv_0 : _MatrixStorage_float4x4_ColMajorstd140_0,
    @align(16) projInv_0 : _MatrixStorage_float4x4_ColMajorstd140_0,
};
@binding(1) @group(0) var<uniform> camera : Camera;

@binding(2) @group(0) var voxel_data : texture_3d<u32>;

@binding(3) @group(0) var<storage, read> palette : array<vec4<f32>>;


@compute
@workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) globalId : vec3<u32>) {
}
