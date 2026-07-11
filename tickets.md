# Tickets: FPS Player Controller with World Collision

Builds a grounded first-person controller with AABB collision against the Tree64
voxel world, gravity, jumping, and fly-mode toggle. Source spec: `plans/007-fps-controller-collision.md`.

Work the **frontier**: any ticket whose blockers are all done. For a purely linear chain that means top to bottom.

## 1. Collision query module (`tree64::query`)

**What to build:** The `aabb_collides` function that performs hierarchical AABB-vs-Tree64 traversal and returns per-axis penetration. Fully tested with synthetic trees — empty space, full block, partial overlap, ground-detection slab shape, AABB at tree boundaries. This ticket has no visible runtime effect; correctness is proven by tests.

**Blocked by:** None — can start immediately.

- [x] `aabb_collides(tree, aabb_min, aabb_max)` returns `CollisionResult::Clear` for an AABB in empty space
- [x] `aabb_collides(tree, aabb_min, aabb_max)` returns `CollisionResult::Blocked` with correct per-axis penetration when the AABB overlaps occupied voxels
- [x] Penetration along +X equals the distance to the nearest non-overlapping position along +X (and same for -X, +Y, -Y, +Z, -Z)
- [x] Hierarchical traversal prunes empty branches: querying an AABB far from the only occupied voxel visits only the root and that voxel's path, not the entire tree
- [x] Ground-detection slabs (thin AABB, one voxel layer thick, full player XZ footprint) correctly detect partial floor support (lattices, beam edges)
- [x] Tree boundaries (AABB extending beyond `tree_dim`) are handled: voxels outside bounds are treated as empty

## 2. Physics loop + gravity + ground

**What to build:** Fixed 60 Hz physics tick with a delta-time accumulator in the main loop. Player falls under gravity (20 m/s²), ground detection via collision query transitions between Grounded/Airborne states. Player spawns in FPS mode, falls onto the nearest solid surface, and stands there. Visible: when you launch the app, the player drops from the starting position and comes to rest on the world floor.

**Blocked by:** 1. Collision query module (`tree64::query`)

- [x] Main loop accumulates frame delta and dispatches physics ticks at 60 Hz before rendering
- [x] Gravity accelerates player downward at 20 m/s² each tick when Airborne
- [x] Ground detection checks the 1-voxel layer directly below the full player XZ footprint each tick
- [x] Player transitions to Grounded when any voxel is found in the ground-detection slab
- [x] Player transitions to Airborne when no voxel is found in the ground-detection slab
- [x] When Grounded, vertical velocity is zeroed and the player's Y position is snapped to the support surface
- [x] Player AABB (0.6 × 1.8 × 0.6 m) does not intersect any occupied voxel at rest on the ground
- [x] Camera renders at player position + 1.65 m eye offset

## 3. WASD movement + wall collision + step-up

**What to build:** WASD keys apply horizontal velocity on the ground (6 m/s, instant accel/decel). Forward/backward directions are derived from the camera yaw, projected onto the horizontal plane. Collision resolution blocks horizontal movement into walls and slides the player along them via depenetration. Obstacles ≤ 0.5 m tall are auto-surmounted with step-up. Visible: you can walk around the world, get stopped by walls, and step over low ledges.

**Blocked by:** 2. Physics loop + gravity + ground

- [x] WASD applies horizontal velocity at 6 m/s when Grounded, following the camera's horizontal facing direction
- [x] Releasing all movement keys stops horizontal velocity immediately on the ground
- [x] Walking forward into a wall stops the player at the wall surface (AABB does not penetrate occupied voxels)
- [x] Walking diagonally into a wall slides along the wall surface rather than stopping entirely
- [x] Swept-AABB stepping (≤ 0.3 m intervals) prevents tunneling through thin walls
- [x] Obstacles ≤ 0.5 m (4 voxels) tall are stepped over automatically when walking forward into them
- [x] Obstacles > 0.5 m tall behave as walls and block movement (require jumping)
- [x] Player cannot walk off the edge of the world — AABB stays within the tree bounds

## 4. Jump + air control + terminal velocity

**What to build:** Space triggers a jump when Grounded, applying 8 m/s upward velocity. In Airborne, WASD provides 50% air control (3 m/s² acceleration). Velocity is clamped at 50 m/s downward and 30 m/s upward terminal velocity. Visible: you can jump, steer while airborne, land, and fall from great heights without exceeding terminal velocity.

**Blocked by:** 3. WASD movement + wall collision + step-up

- [x] Space applies an 8 m/s upward velocity impulse when Grounded and transitions to Airborne
- [x] Space has no effect when already Airborne (no double jump)
- [x] In Airborne, WASD applies horizontal acceleration at 50% of ground rate (3 m/s² effective)
- [x] Airborne horizontal velocity persists without friction (no damping when keys are released mid-air)
- [x] Downward velocity is capped at 50 m/s
- [x] Upward velocity is capped at 30 m/s
- [x] Landing from a jump or fall transitions back to Grounded on the next tick after ground contact

## 5. Fly/FPS toggle with F1

**What to build:** F1 toggles between Fly and FPS control modes. Fly mode preserves the existing flying camera behavior exactly. FPS mode uses the new physics system. Switching snap-freezes: entering Fly zeroes velocity; entering FPS starts Airborne and falls to the ground. Visible: you can toggle between noclip debugging and grounded gameplay with F1.

**Blocked by:** 2. Physics loop + gravity + ground

- [x] F1 toggles between Fly and FPS control modes
- [x] Fly mode replicates the existing flying camera behavior (WASD + Space/Ctrl for full 3D movement, no gravity, no collision)
- [x] FPS mode uses the new physics (gravity, ground detection, collision, jump)
- [x] Switching from FPS to Fly zeroes velocity and keeps the camera at its current position
- [x] Switching from Fly to FPS starts the player Airborne at the current position with zero velocity, then gravity takes over
- [x] Mouse look (yaw/pitch) works identically in both modes
