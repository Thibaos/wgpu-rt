static const float INF = 1e30;
static const float NEG_INF = -1e30;
static const int MAX_STEPS = 768;
static const float CHUNK_SIDE = 255.0;
static const float VOXEL_SCALE = 0.125;
static const float CHUNK_WORLD = CHUNK_SIDE * VOXEL_SCALE;
static const float ENTRY_EPSILON = 1e-4;

[[vk::binding(0, 0)]]
cbuffer CameraData {
  float4 camera_pos;
  float4x4 view_inv;
  float4x4 proj_inv;
  float4x4 view_proj;
  float4 viewport_and_heatmap;
}
camera;

[[vk::binding(0, 1)]]
StructuredBuffer<float4> palette;

[[vk::binding(1, 1)]]
Texture3D<uint> voxel_textures[];

struct VSInput {
  [[vk::location(0)]]
  float4 position;
  [[vk::location(1)]]
  float2 tex_coord;
  [[vk::location(2)]]
  float4 model_matrix_0;
  [[vk::location(3)]]
  float4 model_matrix_1;
  [[vk::location(4)]]
  float4 model_matrix_2;
  [[vk::location(5)]]
  float4 model_matrix_3;
  [[vk::location(6)]]
  float4 chunk_origin_in;
};

struct VertexOutput {
  float4 position : SV_Position;
  float2 tex_coord : TEXCOORD0;
  nointerpolation uint chunk_id : TEXCOORD1;
  nointerpolation float3 chunk_origin : TEXCOORD2;
  float3 world_pos : TEXCOORD3;
};

struct FragmentOutput {
  float4 color : SV_Target0;
  float depth : SV_Depth;
};

[shader("vertex")]
VertexOutput vs_main(VSInput input, uint instance_index: SV_VulkanInstanceID) {
  float4x4 model_matrix =
      transpose(float4x4(input.model_matrix_0, input.model_matrix_1,
                         input.model_matrix_2, input.model_matrix_3));
  VertexOutput result;
  float4 world_pos4 = mul(model_matrix, input.position);
  result.position = mul(camera.view_proj, world_pos4);
  result.tex_coord = input.tex_coord;
  result.chunk_id = instance_index;
  result.chunk_origin = input.chunk_origin_in.xyz;
  result.world_pos = world_pos4.xyz;
  return result;
}

[shader("fragment")]
FragmentOutput fs_main(VertexOutput v) {
  uint vox = voxel_textures[v.chunk_id].Load(int4(0));
  FragmentOutput result;
  float4 clip_pos = mul(camera.view_proj, float4(v.world_pos, 1.0));
  result.color = float4(v.tex_coord.x, v.tex_coord.y, 0.0, 1.0);
  result.depth = clip_pos.z / clip_pos.w;

  if (vox != 0) {
    result.color = palette[vox];
  }

  return result;
}
