// Compute entry point wrapping the Tree64 traversal
// Self-contained — no imports needed

struct HitInfo {
  float3 Pos;
  float3 Normal;
  uint MaterialId;
};

// Count number of set bits in a uint64 below `width`
uint prefix_popcnt64(uint64_t mask, uint width) {
  uint himask = uint(mask);
  uint count = 0;
  if (width >= 32) {
    count = countbits(himask);
    himask = uint(mask >> 32);
  }
  uint m = 1u << (width & 31u);
  count += countbits(himask & (m - 1u));
  return count;
}

// Mirror the existing wgpu-rt bindings
[[vk::binding(0, 0)]]
[format("rgba8")]
RWTexture2D<float4> output;
[[vk::binding(1, 0)]]
cbuffer TreeParams {
  uint rootNodeIndex;
  uint treeScale;
  int3 rootOffset;
};
[[vk::binding(2, 0)]]
cbuffer Camera {
  float3 pos;
  float4x4 viewInv;
  float4x4 projInv;
};
[[vk::binding(3, 0)]]
StructuredBuffer<uint> treeNodes;
[[vk::binding(4, 0)]]
StructuredBuffer<uint> leafData;

// Stub: the original Traversal_OctMirror accesses TreeNodes via a pointer.
// We need to adapt: wrap our storage buffers into a VoxelMap that the Slang
// traversal can use. The original VoxelMap has `Node* TreeNodes` — a raw
// pointer. Since we use storage buffers, we'll implement RayCast manually.

[numthreads(8, 8, 1)]
void main(uint3 globalId: SV_DispatchThreadID) {
  float2 targetSize;
  output.GetDimensions(targetSize.x, targetSize.y);
  float2 pixelCenter = float2(globalId.xy) + 0.5;
  float2 uv = pixelCenter / targetSize;
  float2 ndc = uv * 2.0 - 1.0;

  float3 rayOrigin = mul(viewInv, float4(0, 0, 0, 1)).xyz;
  float4 target = mul(projInv, float4(ndc.x, ndc.y, 1, 1));
  float3 rayDir = normalize(mul(viewInv, float4(normalize(target.xyz), 0)).xyz);

  int3 worldOrigin = int3(floor(rayOrigin)) - rootOffset;
  float3 localPos = frac(rayOrigin);

  // Use the Traversal_OctMirror directly (adapted from tree64.slang)
  HitInfo hit = Tree64RayCast(worldOrigin, localPos, rayDir);

  float4 color = float4(0, 0, 0, 1);
  if (hit.MaterialId != 0) {
    float mat = float(hit.MaterialId) / 255.0;
    color = float4(mat, mat * 0.7, mat * 0.3, 1);

    float3 lightDir = normalize(float3(0.5, 1.0, 0.3));
    float ndotl = max(dot(hit.Normal, lightDir), 0.1);
    color = color * ndotl;
  }

  output[globalId.xy] = color;
}

// --- Adapted traversal that works with storage buffers instead of raw pointers
// ---

struct Node {
  uint packedData0;
  uint packedData1;
  uint packedData2;

  property bool IsLeaf {
    get { return (packedData0 & 1) != 0; }
  }
  property uint ChildPtr {
    get { return packedData0 >> 1; }
  }
  property uint64_t PopMask {
    get { return packedData1 | uint64_t(packedData2) << 32; }
  }
};

Node ReadNode(uint index) {
  uint base = index * 3;
  Node n;
  n.packedData0 = treeNodes[base];
  n.packedData1 = treeNodes[base + 1];
  n.packedData2 = treeNodes[base + 2];
  return n;
}

uint ReadLeafMaterial(Node node, uint childIdx) {
  uint dataOffset = node.ChildPtr;
  uint packedIdx = dataOffset + prefix_popcnt64(node.PopMask, childIdx);
  uint word = leafData[packedIdx / 4];
  uint byteIdx = (packedIdx & 3) * 8;
  return (word >> byteIdx) & 0xFF;
}

// Bit test helper matching the original
bool BitTest(uint64_t value, uint shift, uint mask) {
  uint low = shift < 32 ? uint(value) : uint(value >> 32);
  return (low >> (shift & 31) & mask) != 0;
}

// floor(pos / scale) * scale
float3 FloorScaleLocal(float3 pos, int scaleExp) {
  uint maskVal = ~0u << uint(scaleExp);
  return asfloat(asuint(pos) & maskVal);
}

int GetCellIndex(float3 pos, int scaleExp) {
  uint3 cellPos = asuint(pos) >> uint(scaleExp) & 3;
  return int(cellPos.x + cellPos.z * 4 + cellPos.y * 16);
}

// Mirror position for octant coherence
float3 GetMirroredPosLocal(float3 pos, float3 dir, bool rangeCheck) {
  float3 mirrored = asfloat(asuint(pos) ^ 0x7FFFFF);
  if (rangeCheck && any(pos < 1.0 || pos >= 2.0))
    mirrored = 3.0 - pos;
  float3 result;
  if (dir.x > 0)
    result.x = mirrored.x;
  else
    result.x = pos.x;
  if (dir.y > 0)
    result.y = mirrored.y;
  else
    result.y = pos.y;
  if (dir.z > 0)
    result.z = mirrored.z;
  else
    result.z = pos.z;
  return result;
}

HitInfo Tree64RayCast(int3 worldOrigin, float3 rayPos, float3 rayDir) {
  float scale = 1.0 / float(1 << treeScale);
  float3 origin = float3(worldOrigin) * scale + rayPos * scale + 1.0;

  // Mirror to negative octant for wave coherence
  float3 invDir = 1.0 / -abs(rayDir);
  uint mirrorMask = 0;
  if (rayDir.x > 0)
    mirrorMask |= 3 << 0;
  if (rayDir.z > 0)
    mirrorMask |= 3 << 2;
  if (rayDir.y > 0)
    mirrorMask |= 3 << 4;

  origin = GetMirroredPosLocal(origin, rayDir, true);
  float3 pos = clamp(origin, 1.0, 1.9999999);

  float3 sideDist;
  int childIdx;
  int i;
  bool skipNextHit = true;

  // Clip rays starting outside tree
  if (any(pos != origin)) {
    float3 t0 = (float3(2.0) - origin) * invDir;
    float3 t1 = (float3(1.0) - origin) * invDir;
    float tmin = max(max(t0.x, t0.y), max(t0.z, 0.0));
    float tmax = min(min(t1.x, t1.y), t1.z);

    pos = clamp(origin - abs(rayDir) * tmin, 1.0, 1.9999999);
    sideDist = -t0;
    skipNextHit = tmin >= tmax;
  }

  uint stack[11];
  int scaleExp = 21;

  uint nodeIdx = rootNodeIndex;
  Node node = ReadNode(nodeIdx);

  for (i = 0; i < 256; i++) {
    childIdx = GetCellIndex(pos, scaleExp) ^ int(mirrorMask);

    // Descend
    while (BitTest(node.PopMask, uint(childIdx), 1) && !node.IsLeaf) {
      stack[scaleExp >> 1] = nodeIdx;
      nodeIdx = node.ChildPtr + prefix_popcnt64(node.PopMask, uint(childIdx));
      node = ReadNode(nodeIdx);

      scaleExp -= 2;
      childIdx = GetCellIndex(pos, scaleExp) ^ int(mirrorMask);
    }
    if (BitTest(node.PopMask, uint(childIdx), 1) && node.IsLeaf && !skipNextHit)
      break;

    int advScaleExp = scaleExp;

    // 2³ steps
    if (!BitTest(node.PopMask, uint(childIdx & 0x2A), 0x00330033))
      advScaleExp++;

    float3 edgePos = FloorScaleLocal(pos, advScaleExp);
    sideDist = (edgePos - origin) * invDir;
    float tmax = min(min(sideDist.x, sideDist.y), sideDist.z);

    int3 maxSiblBounds = int3(asint(edgePos));
    if (sideDist.x == tmax)
      maxSiblBounds.x += -1;
    else
      maxSiblBounds.x += (1 << advScaleExp) - 1;
    if (sideDist.y == tmax)
      maxSiblBounds.y += -1;
    else
      maxSiblBounds.y += (1 << advScaleExp) - 1;
    if (sideDist.z == tmax)
      maxSiblBounds.z += -1;
    else
      maxSiblBounds.z += (1 << advScaleExp) - 1;
    pos = min(origin - abs(rayDir) * tmax, asfloat(maxSiblBounds));

    uint3 diffPos = asuint(pos) ^ asuint(edgePos);
    int diffExp =
        firstbithigh((diffPos.x | diffPos.y | diffPos.z) & 0xFFAAAAAA);

    if (diffExp > scaleExp) {
      scaleExp = diffExp;
      if (diffExp > 21)
        break;

      nodeIdx = stack[scaleExp >> 1];
      node = ReadNode(nodeIdx);
    }
    skipNextHit = false;
  }

  HitInfo hit;
  hit.MaterialId = 0;

  if (node.IsLeaf && scaleExp <= 21) {
    pos = GetMirroredPosLocal(pos, rayDir, false);

    hit.MaterialId = ReadLeafMaterial(node, uint(childIdx));
    hit.Pos = pos;

    float tmax = min(min(sideDist.x, sideDist.y), sideDist.z);
    bool3 sideMask = float3(tmax) >= sideDist;
    hit.Normal = float3(0);
    if (sideMask.x)
      hit.Normal.x = -sign(rayDir).x;
    if (sideMask.y)
      hit.Normal.y = -sign(rayDir).y;
    if (sideMask.z)
      hit.Normal.z = -sign(rayDir).z;
  }
  return hit;
}
