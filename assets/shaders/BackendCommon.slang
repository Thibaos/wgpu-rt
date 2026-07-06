public interface IRayCastable {
  public HitInfo RayCast(int3 worldOrigin, float3 origin, float3 dir,
                         bool coarse);
}

public struct HitInfo {
  public float Dist;
  public float3 Pos;
  public float3 Normal;
  public float2 FaceUV;
  public uint MaterialId;

  public property bool Miss {
    get { return MaterialId == 0; }
  }
}

// Count number of set bits in variable range [0..width]
public uint prefix_popcnt64(uint64_t mask, uint width) {
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

// Cheaper approximation of `(value >> shift & mask) != 0` that gives
// correct results for single bits and tile octant tests.
public bool BitTestHalf64(uint64_t value, uint shift, uint mask) {
  uint low = shift < 32 ? uint(value) : uint(value >> 32);
  return (low >> (shift & 31) & mask) != 0;
}
