# Accessing per-chunk voxel data from rasterized AABBs

## Executive answer

A vertex attribute cannot contain a texture or texture view. It can contain a **chunk ID** (or a compact record index), which the vertex shader forwards to the fragment shader as a `flat`/non-interpolated integer. The fragment shader can then use that ID to select voxel data only through a resource organization supported by WebGPU:

1. **Recommended general solution:** store all chunk voxel data in one storage buffer (or a few large buffers), and store per-chunk `offset`, dimensions, and world-space AABB in a second storage buffer. The fragment shader uses the flat chunk ID to look up the record and performs DDA with indexed buffer loads.
2. **If hardware/platform support is acceptable:** use a `binding_array` of 3D texture views, indexed by the flat chunk ID. This requires wgpu's `TEXTURE_BINDING_ARRAY` feature and dynamic-uniform indexing; it is a native-only optional feature, not portable WebGPU baseline functionality.
3. **If chunks have identical dimensions and a bounded atlas is practical:** pack them into one larger 3D texture and use a chunk table to convert `(chunk_id, local_voxel)` to atlas coordinates. This is portable and texture-backed, but wastes space or requires an allocator/packing scheme and careful sampling rules.

The fragment shader cannot take a texture handle from a vertex output, and WebGPU v1 does not provide an array of independent 3D textures through `texture_3d_array`. A `texture_2d_array` is an array of 2D layers, not an array of 3D volumes.

For this project specifically, the existing Tree64 design is likely a better representation than dense per-chunk textures for a large sparse world. A dense 3D texture is attractive for simple DDA, but it pays storage and bandwidth proportional to the chunk volume, not occupied voxels. A storage-buffer chunk pool or Tree64 pool preserves sparse storage while still allowing one draw/dispatch.

## Passing the chunk identity from the AABB draw

Use instancing or an indexed chunk-AABB vertex buffer. The vertex shader receives `@builtin(instance_index)` (or a per-instance vertex attribute), looks up or forwards the chunk index, and passes it to the fragment shader as a non-interpolated integer:

```wgsl
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) chunk_id: u32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VsOut {
    // Construct the appropriate AABB corner from vertex_index.
    // instance_index identifies the chunk.
    var out: VsOut;
    out.position = ...;
    out.chunk_id = instance_index;
    return out;
}
```

The exact `@interpolate(flat)` spelling depends on the WGSL version accepted by the selected wgpu release; the important requirement is integer, non-interpolated interpolation. Do not let a chunk ID be smoothly interpolated across a triangle.

The fragment shader then does a table lookup:

```wgsl
@group(0) @binding(0) var<storage, read> chunks: array<Chunk>;

struct Chunk {
    aabb_min: vec3<f32>,
    aabb_max: vec3<f32>,
    voxel_offset: u32,
    dims_x: u32,
    dims_y: u32,
    dims_z: u32,
};

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    let chunk = chunks[input.chunk_id];
    // Intersect the camera ray with chunk.aabb, then DDA its voxel data.
    ...
}
```

This is the answer to the “vertex data or texture pool?” part: **vertex data carries the identity; it does not carry the resource.** The resource is selected indirectly through a bind group array or an indexed pool.

## Option A: one storage-buffer pool (recommended first implementation)

Pack chunk voxels consecutively into a storage buffer:

```text
voxel_pool = [chunk 0 bytes][chunk 1 bytes][chunk 2 bytes]...
chunk_table[chunk_id] = {
    voxel_byte_offset,
    dimensions,
    world_origin / AABB,
    stride information,
}
```

The shader computes a local integer voxel coordinate after the AABB intersection and reads:

```wgsl
let index = chunk.voxel_offset
    + local.x
    + local.y * chunk.dims_x
    + local.z * chunk.dims_x * chunk.dims_y;
let material = voxel_pool[index];
```

In practice, use a storage-buffer representation whose element type and alignment are unambiguous for WGSL (for example `array<u32>` with four packed material bytes per element, or `array<ChunkVoxel>` with a 32-bit element). A byte-oriented Rust upload format must not be assumed to map directly to a WGSL `array<u8>` unless the target WGSL/wgpu version explicitly supports that storage type and its layout.

Advantages:

- One bind group and one draw.
- No per-chunk texture creation or texture-view lifetime management.
- Arbitrary chunk sizes and a straightforward streaming allocator.
- The chunk ID can index both metadata and voxel storage.
- Works in fragment and compute shaders without bindless texture features.

Disadvantages:

- Loads are buffer loads rather than texture loads.
- You must implement the pool allocator/defragmentation and pack/unpack format.
- A dense chunk still costs `dims.x * dims.y * dims.z` material cells.

For an occupancy-heavy world with compact fixed-size chunks, this is usually the simplest way to validate the AABB + DDA idea before committing to a texture system.

## Option B: array of 3D texture bindings

Native wgpu supports texture binding arrays when the adapter exposes and the application requests `wgpu::Features::TEXTURE_BINDING_ARRAY`. The layout uses `count: Some(...)` on a texture binding, and WGSL declares a binding array, conceptually:

```rust
wgpu::BindGroupLayoutEntry {
    binding: 0,
    visibility: wgpu::ShaderStages::FRAGMENT,
    ty: wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Uint,
        view_dimension: wgpu::TextureViewDimension::D3,
        multisampled: false,
    },
    count: NonZeroU32::new(max_chunk_textures),
}
```

```wgsl
@group(0) @binding(0)
var voxel_textures: binding_array<texture_3d<u32>>;

let material = textureLoad(voxel_textures[input.chunk_id], voxel_coord, 0).r;
```

The wgpu feature source documents this feature as enabling uniform arrays of textures and says they may be indexed by dynamically uniform values. The bind-group source also documents that `count` makes a binding an array and that texture/sampler arrays require `TEXTURE_BINDING_ARRAY`.

Important restrictions and costs:

- This is an optional native feature; the wgpu source lists DX12, Metal, and Vulkan support, and identifies it as native-only. Do not make it the only path if WebGPU portability matters.
- The index must satisfy the platform's dynamic-uniform/resource-indexing rules. A per-fragment index is potentially divergent: two neighboring fragments may refer to different chunks. A shader may compile or run on some native backends but still be a poor portability assumption. Keep a fallback path or use the storage-buffer pool.
- `count` is a fixed binding-array size. It is not an unbounded vector of textures. Query adapter limits and choose a policy for chunks beyond the array capacity.
- Bind-group arrays have resource limits and can make resource lifetime/streaming more complicated. Rebuilding a large bind group whenever chunks stream is expensive.
- Each chunk must have a compatible 3D texture view: same sample type, view dimension, and shader-visible binding type.

This is the closest answer to “a 3D texture pool indexed by chunk,” but it is a **texture binding array**, not a single 3D texture pool. If you use it, the AABB instance ID should be the binding-array index.

## Option C: 3D texture atlas

Create one large `texture_3d` and tile chunk volumes into it. A chunk table stores the atlas origin and dimensions:

```text
chunk_table[id].atlas_origin = (ox, oy, oz)
chunk_table[id].dims = (sx, sy, sz)
atlas_voxel = atlas_origin + local_voxel
```

The shader performs one normal `textureLoad` from the atlas. This is supported by baseline texture bindings and avoids bindless features.

However, it is not a free “array of 3D textures”:

- The atlas has one global width/height/depth limit and must fit the adapter's 3D texture limits.
- Fixed-size chunks can waste large amounts of space when mostly empty or when only a few chunks are loaded.
- Adjacent tiles can bleed into each other if filtering is used. For integer `textureLoad` at exact texel coordinates this is less problematic, but keep one-voxel borders if using filtered sampling or mipmaps.
- Dynamic chunk insertion needs a 3D free-space allocator or a fixed atlas layout.
- A single huge texture can create allocation and upload stalls, so a multi-atlas scheme may still be necessary.

An atlas is appropriate when chunks are uniform, the active working set is bounded, and texture-cache behavior is more important than sparse memory usage.

## Option D: one global 3D texture

If the world dimensions fit the device's 3D texture limits and dense storage is acceptable, flatten the world into one volume. Then no chunk ID is needed for voxel lookup; the AABB ID can still identify metadata for culling and world-to-volume transforms.

This is the simplest shader, but it is generally unsuitable for the project's very large sparse world. A volume's memory and upload cost scale with the complete bounding box, including empty space. The existing project plans already identify this problem for large scenes.

## What AABB rasterization does and does not provide

Rasterizing one AABB per chunk can provide a screen-space candidate mask: only pixels covered by a projected chunk box execute that chunk's fragment shader. It does not automatically provide ideal ray traversal or occlusion culling.

### Correctness concerns

1. **A box has multiple faces.** Drawing all six faces can invoke the fragment shader multiple times per pixel. Back-face culling and drawing only front faces can reduce this, but a camera inside a chunk needs a separate policy.
2. **Projected AABBs overlap.** Overlapping chunk boxes produce overlapping fragments. A depth test based on the AABB surface chooses the nearest box surface, not necessarily the nearest occupied voxel hit. A farther box may contain the first visible voxel when the nearer box is empty along that ray.
3. **Discard and depth writes matter.** If a DDA miss discards, later overlapping chunks may still need to run. If it writes a depth value for the voxel hit, the depth must correspond to the actual hit, not the rasterized AABB face. Treating AABB depth as visibility can incorrectly cull geometry.
4. **Occlusion is not free.** A chunk's AABB being covered by an already-rendered AABB does not prove that its voxel geometry is occluded. You need a hierarchical-Z/previous-depth test, a conservative occlusion test, or a separate visibility pass.
5. **Thin/edge-on boxes and precision.** Small projected AABBs can be missed or cover unstable pixel regions. Camera-inside and near-plane cases require explicit ray/AABB handling.

### Performance concern

If every chunk projects over much of the screen, AABB rasterization causes roughly `number_of_chunks × covered_pixels` fragment work, and each invocation runs a DDA. For many overlapping chunks this can be worse than one ray per screen pixel traversing a world-level structure.

A common architecture is therefore:

1. GPU frustum-cull chunk AABBs (compute or indirect draw).
2. Build a compact visible-chunk list.
3. Render each screen pixel once in a compute shader.
4. Traverse a world/chunk acceleration structure, then the selected chunk's voxel representation.

If using rasterization, use it first as a **frustum/candidate culling experiment**, not as an assumed occlusion solution. A depth prepass or hierarchical-Z pass can be added after correctness is established.

## Recommended design for this repository

### Short-term prototype

Keep the chunk AABB raster pass, but use it only to test the data flow:

- Instance one box per chunk.
- Pass `@builtin(instance_index)` through a flat `u32 chunk_id`.
- Bind a `ChunkTable` storage buffer and a packed voxel storage buffer.
- Perform ray/AABB intersection and DDA in the fragment shader.
- Disable assumptions that AABB depth equals voxel depth; resolve visibility using the DDA hit distance.
- Start with one chunk per draw or a deliberately small visible set if bindless indexing is not available.

This gives a portable baseline and makes the chunk metadata path explicit.

### Scalable production direction

Prefer a two-level sparse traversal:

- A world-level chunk occupancy structure (the existing Tree64 approach is a strong candidate) finds candidate chunks along the ray.
- Each chunk uses a compact Tree64 or a dense representation only if its occupancy justifies it.
- A compute shader casts one ray per output pixel and performs nearest-hit selection across chunks.

If a dense texture is required for simplicity, use either:

- a storage-buffer pool for portable, dynamically streamed chunks; or
- a 3D atlas for a bounded fixed-size active set.

Use `TEXTURE_BINDING_ARRAY` only as an optional native fast path after checking adapter features and limits. Do not design the file/world format around a permanently fixed number of texture bindings.

## Sources

- wgpu feature definitions (`TEXTURE_BINDING_ARRAY`, supported backends and dynamic-uniform indexing): https://github.com/gfx-rs/wgpu/blob/trunk/wgpu-types/src/features.rs
- wgpu bind-group binding-array `count` rules and feature requirements: https://github.com/gfx-rs/wgpu/blob/trunk/wgpu-types/src/binding.rs
- wgpu texture view dimensions (`D2Array`, `D3`, etc.): https://docs.rs/wgpu/latest/wgpu/enum.TextureViewDimension.html
- wgpu texture creation and 3D texture API: https://docs.rs/wgpu/latest/wgpu/struct.TextureDescriptor.html
- wgpu bind-group layout API: https://docs.rs/wgpu/latest/wgpu/struct.BindGroupLayoutEntry.html
- WebGPU specification: https://www.w3.org/TR/webgpu/
- WGSL specification (resource variables, interpolation, texture built-ins): https://gpuweb.github.io/gpuweb/wgsl/
- wgpu project guidance on consolidating resources and using atlases/arrays rather than creating many small resources per frame: https://github.com/gfx-rs/wgpu/wiki/Do%27s-and-Dont%27s
