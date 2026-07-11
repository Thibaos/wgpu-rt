# 007 — FPS Player Controller with World Collision

## Problem Statement

The current player controller is a flying noclip camera. There is no gravity, no
ground, and no collision with the voxel world. The user can fly through walls
and cannot experience the world as a first-person character. To build any
gameplay on top of this engine, a grounded first-person controller with physics
and collision against the voxel world is needed.

## Solution

An FPS player controller that replaces the flying camera. The player has an
AABB collider, falls under gravity, stands on solid voxels, walks on the ground
with WASD input, jumps with Space, and resolves collisions against the Tree64
world using hierarchical traversal. A fixed 60 Hz physics tick decouples
movement from render frame rate. A fly mode toggle (F1) is retained as a debug
convenience.

## User Stories

1. As a player, I want to fall under gravity when there is no ground beneath me,
   so that the world feels physically grounded.

2. As a player, I want to land on solid voxels and stop falling, so that I can
   stand on floors and platforms.

3. As a player, I want to walk on the ground using WASD keys, so that I can
   explore the world horizontally.

4. As a player, I want movement to stop immediately when I release all
   movement keys on the ground, so that the controls feel responsive.

5. As a player, I want to jump with the Space key when standing on the ground,
   so that I can reach higher surfaces.

6. As a player, I want limited mid-air steering at 50% of ground acceleration,
   so that I can correct my trajectory while jumping or falling.

7. As a player, I want my horizontal movement to be blocked by solid walls,
   so that I cannot walk through geometry.

8. As a player, I want my vertical movement to be blocked by ceilings,
   so that I cannot jump through solid roof voxels.

9. As a player, I want to automatically step up onto low obstacles up to 0.5 m
   (4 voxels) tall while walking forward, so that minor terrain variation
   doesn't block movement.

10. As a player, I want obstacles taller than 0.5 m to behave as walls, so
    that climbing requires jumping.

11. As a player, I want to walk across lattice floors and trench edges without
    falling through small gaps, so that partial ground support is sufficient
    to stand on.

12. As a player, I want to fall off ledges when there is no voxel anywhere
    under my footprint, so that edges and pits behave naturally.

13. As a player, I want my falling speed to cap at terminal velocity, so that
    long falls don't produce absurd speeds.

14. As a player, I want physics to be smooth and deterministic regardless of
    my frame rate, so that movement feels consistent even during frame drops.

15. As a developer, I want to toggle between FPS mode and the original fly mode
    with F1, so that I can debug the world from a noclip perspective.

16. As a player, I want my camera positioned at eye height (1.65 m) above my
    feet, so that the first-person perspective is correctly scaled.

## Implementation Decisions

### Physics model

Velocity-based with two states: Grounded and Airborne. The player tracks a
velocity vector separate from position. In the Grounded state, horizontal input
applies instant acceleration to max ground speed; releasing keys stops
immediately. Jump applies an upward velocity impulse and transitions to
Airborne. In Airborne, gravity accelerates downward each tick, keyboard input
applies at 50% of ground acceleration, and velocity is not damped. Landing
detection transitions back to Grounded.

### Fixed timestep

Physics runs at 60 Hz (≈16.7 ms per tick), decoupled from render rate via a
delta-time accumulator in the main loop. Each frame: accumulate wall-clock
delta, run `floor(accumulated / tick_duration)` physics ticks, render once.

### Collider

Axis-aligned bounding box: 0.6 × 1.8 × 0.6 m (5 × 14 × 5 voxels). Represented
in world-space meters. The camera is offset +1.65 m (13 voxels) from the AABB
bottom for first-person rendering.

### Collision query

A new module `tree64::query` contains a free function:

`fn aabb_collides(tree: &GpuTree64, aabb_min: Vec3, aabb_max: Vec3) -> CollisionResult`

where `CollisionResult` is:

```rust
enum CollisionResult {
    Clear,
    Blocked {
        penetration_x: f32,
        penetration_neg_x: f32,
        penetration_y: f32,
        penetration_neg_y: f32,
        penetration_z: f32,
        penetration_neg_z: f32,
    },
}
```

Each penetration value is the minimum distance along that axis-axis direction
to clear all overlapping occupied voxels. If no voxels overlap, the result is
Clear.

The function performs hierarchical AABB-vs-tree traversal. Starting at the root
node, it computes the child cell AABB for each set bit in `pop_mask`, tests
overlap with the query AABB, and recursively descends into overlapping children.
At leaf nodes, it tests individual occupied bits for overlap and accumulates
per-axis penetration.

### Collision resolution

Iterative depenetration with axis priority Y → X → Z. For each candidate
position after velocity integration:

1. Test a swept AABB along the displacement vector in steps of ≤ 0.3 m
2. At each step, query collision at the AABB position
3. If Blocked: try pushing out along +Y, then -Y, then +X, -X, +Z, -Z,
   selecting the minimal correction that clears all voxels
4. If step-up is enabled (horizontal movement is blocked but Y-resolved):
   test the AABB raised by step height (0.5 m). If clear, snap to that
   elevated position and retry horizontals
5. Apply the resolved position

### Ground detection

The player is Grounded when at least one occupied voxel exists anywhere within
a 1-voxel layer directly below the player's full XZ footprint (0.6 × 0.6 m).
If no voxel occupies that layer, the player is Airborne. This tolerates
lattices, beam edges, and trenches narrower than the AABB.

### Step height

Vertical obstacles ≤ 0.5 m (4 voxels) are auto-surmounted. The collision
resolver raises the candidate position by the step height and retries horizontal
resolution. Obstacles above 4 voxels are walls.

### Gravity and jump

Gravity: 20 m/s² downward. Jump impulse: 8 m/s upward, producing ~1.6 m jump
height (~13 voxels). Terminal velocity: 50 m/s downward, 30 m/s upward.

### Ground movement

Max ground speed: 6 m/s. Acceleration and deceleration are instantaneous on the
ground — no inertia, no ice physics. In air, acceleration is 50% of ground.

### Player controller structure

A single `PlayerController` struct with an internal `ControlMode` enum:

```rust
enum ControlMode {
    Fly,
    Fps,
}
```

Yaw, pitch, sensitivity, and translation are shared regardless of mode. In Fly
mode, movement applies directly to translation (as today). In FPS mode,
movement is driven by velocity and resolved through physics ticks. F1 toggles
between modes, zeroing velocity on entry to Fly and starting Airborne on entry
to FPS.

### Input bindings

FPS mode: Space = Jump (only when Grounded). Shift and Ctrl have no effect.
WASD = horizontal movement. Mouse = look (unchanged from fly mode).

### Game loop

The main loop (in the framework module) gains a tick accumulator. Events are
processed first, then physics ticks are drained from the accumulator, then the
frame is rendered. The App receives a new `physics_tick` method that delegates
to the player controller, passing the tree reference from the loaded World.

### Tree access from controller

The player controller does not own the tree. Each `physics_tick` call receives
`&GpuTree64` (or `None` for an empty world). The collision query is a free
function in `tree64::query` with no coupling to the player controller.

## Testing Decisions

### What makes a good test

Tests assert external behavior: "given this tree and this AABB position, what
is the collision result?" and "given this tree and these inputs over N ticks,
what is the player's final position?" Implementation details like node traversal
order or accumulator precision are not tested directly.

### Modules tested

- `tree64::query::aabb_collides` — the primary seam. Tested with synthetic
  `GpuTree64` instances built via `GpuNode::new` and hand-assembled node/leaf
  arrays. Covers: empty space, full block, partial overlap, AABB at tree
  boundaries, ground-detection slab shape.
- Player controller state machine — tested via public API (`physics_tick`,
  view matrix output, grounded state) with synthetic trees. Covers: gravity
  integration, ground detection transitions, jump mechanics, wall blocking,
  step-up, air control, fly/FPS toggle.

### Prior art

The existing `tree64::builder` test suite uses the same pattern: construct a
`GpuTree64` from known voxel data via `build_gpu_tree`, then call a lookup
function and assert results. The collision tests follow this convention.

## Out of Scope

- Crouching (reduced AABB height)
- Sprinting (increased ground speed)
- Slope / stair handling beyond step-up
- Multiplayer or networked physics
- Player damage, health, or fall damage
- World modification (adding/removing voxels at runtime)
- Controller support (gamepad input)
- Head bob or view bobbing animations

## Further Notes

The Tree64 is the sole collision representation — there is no separate physics
mesh or simplified collider. This is an architectural decision recorded in ADR
0001. The `.world` format is unchanged; no new serialized data is required for
physics.
