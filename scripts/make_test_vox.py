#!/usr/bin/env python3
"""Create a minimal .vox file (8x8x8, a solid 4x4x4 cube in the center)."""
import struct

def make_vox(path):
    voxels = []
    for x in range(2, 6):
        for y in range(2, 6):
            for z in range(2, 6):
                voxels.append(struct.pack('BBBB', x, y, z, 1))

    size_chunk_data = struct.pack('<III', 8, 8, 8)
    size_chunk = struct.pack('<I', 4)  # chunk id offset (will fill later)
    # We'll fill properly:
    # PACK chunk: id='PACK', size=4, children=0, data=num_models(=1)
    # SIZE chunk: id='SIZE', size=12, children=0, data=(8,8,8)
    # XYZI chunk: id='XYZI', size=4+len(voxels)*4, children=0, data=num_voxels + voxels

    voxel_data = struct.pack('<I', len(voxels)) + b''.join(voxels)
    xyzi_chunk_data = voxel_data

    # Build chunks (no PACK needed since we have 1 model)
    size_chunk = b'SIZE' + struct.pack('<II', 12, 0) + struct.pack('<III', 8, 8, 8)
    xyzi_chunk = b'XYZI' + struct.pack('<II', len(voxel_data), 0) + voxel_data

    main_data = size_chunk + xyzi_chunk
    main_chunk = b'MAIN' + struct.pack('<II', 0, len(main_data)) + main_data

    with open(path, 'wb') as f:
        f.write(b'VOX ')
        f.write(struct.pack('<I', 150))
        f.write(main_chunk)

    print(f"Created {path} with {len(voxels)} voxels")

make_vox('assets/test_scene.vox')
