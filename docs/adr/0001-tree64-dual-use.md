# Tree64 used for both GPU ray tracing and CPU collision queries

The Tree64 hierarchical 4³ voxel structure serves double duty: it is the
acceleration structure for GPU compute-shader ray traversal, and it is also the
sole collision representation queried by the CPU-side FPS player controller.

Rejected alternative: a separate collision mesh or volume built independently
from the voxel data. This was rejected because it would duplicate the world
representation and introduce synchronization risk — the GPU-rendered world and
the physics world could diverge. The Tree64 is already resident on the CPU
side (it is serialized to .world, loaded, then uploaded to GPU buffers), so
collision queries add no extra data or build step.
