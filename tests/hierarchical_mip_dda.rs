//! Deterministic CPU reference and oracle for the hierarchical mip DDA.
//!
//! The GPU shader (`assets/shaders/chunk.wgsl`) traverses a six-frame stack:
//! mip 5 (8^3 root grid) down to mip 0, with bounded descent. This test file
//! implements two *independent* CPU models of that traversal so the difficult
//! interval, negative-boundary, tie, and sibling-recovery rules are testable
//! without a GPU:
//!
//! - `traverse` is the hierarchical reference: it mirrors the shader's
//!   observable rules (root mip 5 grid 8, refinement grids 2, six frames,
//!   explicit texture origin, parent advanced before child push, half-open
//!   intervals, negative-boundary correction, all-axis tie advancement,
//!   sibling recovery). It intentionally applies NO shader caps (24/8/2048);
//!   those are shader safety bounds, not algorithm behavior.
//! - `oracle` is an independent direct mip-0 DDA with its own ray/AABB

//!   entry/exit and an analytical termination bound of `3 * BASE_SIZE + 1`
//!   positive-width cell iterations. It samples only `levels[0]`.
//!
//! Coordinates are normalized chunk-local `[0, 1]^3`. A mip-0 voxel `(x,y,z)`
//! occupies `[x/256, (x+1)/256) x [y/256, (y+1)/256) x [z/256, (z+1)/256)`.
//! `t` is the normalized ray parameter from the ray origin to the mip-0 cell
//! entry. Traversal is half-open: intervals are `[t_enter, t_exit)` and a
//! non-zero coarse value is only an occupancy hint, never a color or a hit.

use std::collections::HashMap;

use glam::{IVec3, Vec3};

const BASE_SIZE: i32 = 256;
const ROOT_MIP: u32 = 5;
const ROOT_GRID: i32 = 8;
/// Normalized-coordinate comparison epsilon: 1e-6 m / 32 m (the shader's
/// `T_EPS` metre epsilon expressed in chunk-normalized units).
const EPS: f32 = 3.125e-8;
/// Normalized zero-ray threshold: 1e-8 m / 32 m. A direction with
/// `dot(dir, dir) <= RAY_LEN_EPS * RAY_LEN_EPS` is a miss.
const RAY_LEN_EPS: f32 = 3.125e-10;
/// Tolerance for the reference-vs-oracle `t` comparison. The two
/// implementations are intentionally independent: the reference derives
/// boundary times through nested frame bounds (`bounds_min + cell * cell_size`
/// at every level), while the oracle computes them from mip-0 indices directly
/// (`(c + 1) * (1/256)`). f32 rounding between these paths accumulates to
/// ~2e-7 on diagonal rays (measured: 0.5150401 vs 0.5150403), far above the
/// 3.125e-8 comparison epsilon. This is a float-path tolerance, not a
/// correctness epsilon: the GPU shader uses a single code path and cannot
/// disagree with itself. The hand-verified expected values in the tests still
/// use `EPS`.
const T_TOLERANCE: f32 = 1e-6;
/// Direction-component threshold below which an axis is treated as parallel
/// (mirrors the shader's `PARALLEL_EPS` on unit directions).
const PARALLEL_EPS: f32 = 1e-8;
/// Oracle analytical termination bound: `3 * BASE_SIZE + 1` positive-width
/// cell-processing iterations (a ray crosses at most 256 cells per axis).
const ORACLE_BOUND: i32 = 3 * BASE_SIZE + 1;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Hit {
    material: u8,
    t: f32,
    voxel: IVec3,
}

/// Sparse per-mip occupancy maps, or the compact logically-full fixture.
///
/// `Sparse(levels)`: `levels[0]` is mip 0, `levels[5]` is mip 5; missing keys
/// are zero/air. Mip `m` has logical size `256 >> m`.
///
/// `Full(material)`: every queried cell at every mip (0 through 5) returns
/// `material`. This is safe only because traversal queries are confined to the
/// ray path by construction; the fixture never materializes the whole volume.
/// It must not be used for malformed-level, mapping, or generation tests.
#[derive(Clone)]
enum Fixture {
    Sparse(Vec<HashMap<IVec3, u8>>),
    Full(u8),
}

impl Fixture {
    fn sparse(levels: Vec<HashMap<IVec3, u8>>) -> Self {
        assert_eq!(levels.len(), 6, "need exactly levels 0..=5");
        Self::Sparse(levels)
    }

    const fn full(material: u8) -> Self {
        Self::Full(material)
    }

    /// Builds levels 1..=5 from sparse mip-0 cells using the same 2^3
    /// occupancy rule as the CPU chunk path: a coarser cell stores the first
    /// non-zero material found in its 2x2x2 children. Coarse materials are
    /// occupancy witnesses only and must never be used as a rendered material.
    fn generated(mip0: &HashMap<IVec3, u8>) -> Self {
        let mut levels = Vec::with_capacity(6);
        levels.push(mip0.clone());
        for m in 1..=ROOT_MIP {
            let mut level = HashMap::new();
            for (child, &mat) in &levels[(m - 1) as usize] {
                if mat != 0 {
                    level.insert(*child / 2, mat);
                }
            }
            levels.push(level);
        }
        Self::Sparse(levels)
    }

    fn sample(&self, mip: u32, coord: IVec3) -> u8 {
        match self {
            Self::Full(mat) => *mat,
            Self::Sparse(levels) => levels[mip as usize].get(&coord).copied().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Ray / AABB helper
// ---------------------------------------------------------------------------

/// Slab ray/AABB span `(t_enter, t_exit)` for a half-open traversal, or `None`
/// on miss. Mirrors the shader's `ray_aabb`: entry clamped to >= 0, miss when
/// the box is entirely behind the origin or the span is empty.
fn ray_aabb(origin: Vec3, dir: Vec3, bmin: Vec3, bmax: Vec3) -> Option<(f32, f32)> {
    let mut t_enter = 0.0f32;
    let mut t_exit = f32::INFINITY;
    for i in 0..3 {
        let o = origin[i];
        let d = dir[i];
        let lo = bmin[i];
        let hi = bmax[i];
        if d.abs() < PARALLEL_EPS {
            // Parallel to this slab: hit only if the origin is inside it.
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d;
        let t1 = (lo - o) * inv;
        let t2 = (hi - o) * inv;
        t_enter = t_enter.max(t1.min(t2));
        t_exit = t_exit.min(t1.max(t2));
    }
    if t_exit < t_enter || t_exit < 0.0 {
        return None;
    }
    Some((t_enter.max(0.0), t_exit))
}

// ---------------------------------------------------------------------------
// Hierarchical reference
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Frame {
    mip: u32,
    grid_size: i32,
    tex_origin: IVec3,
    bounds_min: Vec3,
    bounds_max: Vec3,
    interval: (f32, f32),
    cell: IVec3,
    t: f32,
    t_max: Vec3,
    t_delta: Vec3,
    axis_step: IVec3,
    steps_taken: i32,
}

/// Creates a frame whose first sample uses `interval.0` (the interval entry
/// parameter), never `min(t_max)` (which is the cell exit).
///
/// Applies direction-aware negative-boundary correction before `floor`: when
/// the ray component is negative and the normalized coordinate is within the
/// comparison epsilon of an integer boundary, the floored cell is decremented
/// on that axis before clamping. Positive-direction boundary entries select
/// the cell on the positive side. Every axis is clamped to
/// `[0, grid_size - 1]`.
//
// Eight parameters mirror the WGSL `init_frame` of the same signature.
#[allow(clippy::too_many_arguments)]
fn init_frame(
    origin: Vec3,
    dir: Vec3,
    mip: u32,
    grid_size: i32,
    tex_origin: IVec3,
    bounds_min: Vec3,
    bounds_max: Vec3,
    interval: (f32, f32),
) -> Frame {
    let cell_size = (bounds_max - bounds_min) / grid_size as f32;
    let entry = origin + dir * interval.0;
    let local = (entry - bounds_min) / cell_size;

    let mut cell = IVec3::ZERO;
    let mut t_max = Vec3::splat(f32::INFINITY);
    let mut t_delta = Vec3::splat(f32::INFINITY);
    let mut axis_step = IVec3::ZERO;

    for i in 0..3 {
        let d = dir[i];
        let mut c = local[i].floor() as i32;
        let near_boundary = (local[i] - local[i].round()).abs() * cell_size[i] <= EPS;
        if d < 0.0 && near_boundary {
            c -= 1;
        }
        c = c.clamp(0, grid_size - 1);
        cell[i] = c;

        if d.abs() < PARALLEL_EPS {
            t_max[i] = f32::INFINITY;
            t_delta[i] = f32::INFINITY;
            axis_step[i] = 0;
        } else {
            axis_step[i] = if d > 0.0 { 1 } else { -1 };
            // Distance between cell boundaries along this axis in ray-param
            // units (cell size / |dir component|).
            t_delta[i] = cell_size[i] / d.abs();
            // Absolute ray parameter of the next boundary on this axis.
            let boundary = if axis_step[i] > 0 {
                bounds_min[i] + (c + 1) as f32 * cell_size[i]
            } else {
                bounds_min[i] + c as f32 * cell_size[i]
            };
            t_max[i] = (boundary - origin[i]) / d;
        }
    }

    Frame {
        mip,
        grid_size,
        tex_origin,
        bounds_min,
        bounds_max,
        interval,
        cell,
        t: interval.0,
        t_max,
        t_delta,
        axis_step,
        steps_taken: 0,
    }
}

/// Advances the frame to the raw minimum of the three `t_max` values, then
/// advances *every* axis whose boundary is within `EPS` of that minimum
/// (all-axis tie behavior). Parallel axes (step 0, `t_max = INF`) are never
/// advanced by this rule because `INF - min_t` exceeds any epsilon.
fn advance_frame(mut frame: Frame, _dir: Vec3) -> Frame {
    let min_t = frame.t_max.min_element();
    frame.t = min_t;
    for i in 0..3 {
        if frame.t_max[i] - min_t <= EPS {
            frame.cell[i] += frame.axis_step[i];
            frame.t_max[i] += frame.t_delta[i];
        }
    }
    frame
}

/// Hierarchical six-frame traversal. Mirrors the shader's observable rules;
/// intentionally applies no 24/8/2048 caps. Termination is structural: each
/// frame's interval shrinks with every advance, a ray crosses at most 4 cells
/// of a 2^3 grid and at most 22 of the 8^3 root grid, and every descent ends
/// at mip 0.
fn traverse(origin: Vec3, dir: Vec3, fixture: &Fixture) -> Option<Hit> {
    // Zero-length (or near-zero) ray: miss. Normalized equivalent of the
    // shader's 1e-8 m ray-length threshold.
    if dir.length_squared() <= RAY_LEN_EPS * RAY_LEN_EPS {
        return None;
    }
    let span = ray_aabb(origin, dir, Vec3::ZERO, Vec3::ONE)?;
    // Half-open span: reject point-only edge/corner contact.
    if span.1 - span.0 <= EPS {
        return None;
    }

    let mut frames: Vec<Frame> = vec![init_frame(
        origin,
        dir,
        ROOT_MIP,
        ROOT_GRID,
        IVec3::ZERO,
        Vec3::ZERO,
        Vec3::ONE,
        span,
    )];

    loop {
        if frames.is_empty() {
            return None;
        }
        let top_idx = frames.len() - 1;
        let top = frames[top_idx];

        // Pop: interval exhausted (entry at/after the exclusive exit, or a
        // non-positive-width interval).
        let interval_empty = top.interval.1 - top.interval.0 <= EPS;
        let at_or_after_exit = top.t >= top.interval.1 - EPS;
        if interval_empty || at_or_after_exit {
            frames.pop();
            continue;
        }

        // Current cell's raw exit: nearest boundary vs the exclusive exit.
        let next_boundary = top.t_max.min_element();
        let cell_exit = next_boundary.min(top.interval.1);
        let width = cell_exit - top.t;

        if width <= EPS {
            // Zero-width interval: do not sample; advance (the loop top then
            // pops when the advance reaches the exclusive exit).
            frames[top_idx] = advance_frame(top, dir);
            continue;
        }

        // Positive-width sample.
        let mut top = top;
        top.steps_taken += 1;
        let coord = top.tex_origin + top.cell;
        let in_range = coord.cmpge(IVec3::ZERO).all() && coord.cmplt(IVec3::splat(256)).all();
        let mat = if in_range {
            fixture.sample(top.mip, coord)
        } else {
            0
        };

        if top.mip == 0 {
            // Mip 0 supplies material, entry depth, and the hit voxel. A zero
            // value here is not a miss: it advances exactly like mip 1..5.
            if mat != 0 {
                return Some(Hit {
                    material: mat,
                    t: top.t,
                    voxel: coord,
                });
            }
            frames[top_idx] = advance_frame(top, dir);
            continue;
        }

        if mat != 0 {
            // Coarse occupancy only: capture the parent cell before advancing,
            // advance the parent (front-to-back sibling order), then push the
            // bounded 2^3 child.
            let parent_entry = top.t;
            let parent_next = next_boundary;
            let child_exit = parent_next.min(top.interval.1);
            let cell_size = (top.bounds_max - top.bounds_min) / top.grid_size as f32;
            let cell_bmin = top.bounds_min + top.cell.as_vec3() * cell_size;
            let cell_bmax = cell_bmin + cell_size;
            let child_tex_origin = 2 * (top.tex_origin + top.cell);
            let child_mip = top.mip - 1;

            let advanced = advance_frame(top, dir);
            frames[top_idx] = advanced;

            if child_exit - parent_entry > EPS {
                frames.push(init_frame(
                    origin,
                    dir,
                    child_mip,
                    2,
                    child_tex_origin,
                    cell_bmin,
                    cell_bmax,
                    (parent_entry, child_exit),
                ));
            }
        } else {
            frames[top_idx] = advance_frame(top, dir);
        }
    }
}

// ---------------------------------------------------------------------------
// Independent direct mip-0 oracle
// ---------------------------------------------------------------------------

/// Direct mip-0 DDA with its own ray/AABB entry/exit and an analytical
/// termination bound of `3 * BASE_SIZE + 1` positive-width cell iterations.
/// Zero-width intervals do not consume the bound. Samples only `levels[0]`
/// and returns the first non-zero material, entry `t`, and voxel coordinate.
/// If the bound is exhausted without a hit, returns `None`.
fn oracle(origin: Vec3, dir: Vec3, mip0: &HashMap<IVec3, u8>) -> Option<Hit> {
    if dir.length_squared() <= RAY_LEN_EPS * RAY_LEN_EPS {
        return None;
    }
    let span = ray_aabb(origin, dir, Vec3::ZERO, Vec3::ONE)?;
    if span.1 - span.0 <= EPS {
        return None;
    }

    let cell_size = 1.0 / BASE_SIZE as f32;
    let entry = origin + dir * span.0;
    let local = entry / cell_size;

    let mut cell = IVec3::ZERO;
    let mut t_max = Vec3::splat(f32::INFINITY);
    let mut t_delta = Vec3::splat(f32::INFINITY);
    let mut axis_step = IVec3::ZERO;
    for i in 0..3 {
        let d = dir[i];
        let mut c = local[i].floor() as i32;
        let near_boundary = (local[i] - local[i].round()).abs() * cell_size <= EPS;
        if d < 0.0 && near_boundary {
            c -= 1;
        }
        c = c.clamp(0, BASE_SIZE - 1);
        cell[i] = c;
        if d.abs() < PARALLEL_EPS {
            t_max[i] = f32::INFINITY;
            t_delta[i] = f32::INFINITY;
            axis_step[i] = 0;
        } else {
            axis_step[i] = if d > 0.0 { 1 } else { -1 };
            t_delta[i] = cell_size / d.abs();
            let boundary = if axis_step[i] > 0 {
                (c + 1) as f32 * cell_size
            } else {
                c as f32 * cell_size
            };
            t_max[i] = (boundary - origin[i]) / d;
        }
    }

    let mut t = span.0;
    let mut processed = 0;
    while processed < ORACLE_BOUND {
        let next_boundary = t_max.min_element();
        let cell_exit = next_boundary.min(span.1);
        let width = cell_exit - t;
        if width <= EPS {
            // Zero-width: advance without sampling and without consuming the
            // positive-width iteration bound.
            t = next_boundary;
            for i in 0..3 {
                if t_max[i] - next_boundary <= EPS {
                    cell[i] += axis_step[i];
                    t_max[i] += t_delta[i];
                }
            }
            if t >= span.1 - EPS {
                break;
            }
            continue;
        }

        processed += 1;
        let in_range = cell.cmpge(IVec3::ZERO).all() && cell.cmplt(IVec3::splat(BASE_SIZE)).all();
        if in_range
            && let Some(&mat) = mip0.get(&cell)
            && mat != 0
        {
            return Some(Hit {
                material: mat,
                t,
                voxel: cell,
            });
        }
        t = next_boundary;
        for i in 0..3 {
            if t_max[i] - next_boundary <= EPS {
                cell[i] += axis_step[i];
                t_max[i] += t_delta[i];
            }
        }
        if t >= span.1 - EPS {
            break;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Runs both implementations and asserts they agree on material, voxel, and
/// entry parameter within the documented epsilon.
fn assert_agree(fixture: &Fixture, mip0: &HashMap<IVec3, u8>, origin: Vec3, dir: Vec3) {
    let hierarchical = traverse(origin, dir, fixture);
    let direct = oracle(origin, dir, mip0);
    match (hierarchical, direct) {
        (None, None) => {}
        (Some(h), Some(o)) => {
            assert_eq!(h.voxel, o.voxel, "voxel mismatch for ray {origin} {dir}");
            assert_eq!(
                h.material, o.material,
                "material mismatch for ray {origin} {dir}"
            );
            assert!(
                (h.t - o.t).abs() <= T_TOLERANCE,
                "t mismatch: hierarchical {} vs oracle {} for ray {origin} {dir}",
                h.t,
                o.t
            );
        }
        (h, o) => panic!(
            "reference/oracle disagreement for ray {origin} {dir}: hierarchical={h:?} oracle={o:?}"
        ),
    }
}

fn fixture_with(mip0: &HashMap<IVec3, u8>) -> (Fixture, HashMap<IVec3, u8>) {
    (Fixture::generated(mip0), mip0.clone())
}

// ---------------------------------------------------------------------------
// Named regression cases
// ---------------------------------------------------------------------------

#[test]
fn empty_chunk_is_a_miss() {
    let mip0 = HashMap::new();
    let levels = vec![HashMap::new(); 6];
    let fixture = Fixture::sparse(levels);
    let origin = Vec3::new(0.5, 0.5, 0.5);
    for dir in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Z, Vec3::ONE.normalize()] {
        assert_eq!(traverse(origin, dir, &fixture), None);
        assert_eq!(oracle(origin, dir, &mip0), None);
    }
}

#[test]
fn fully_occupied_ray_path_returns_nearest_voxel() {
    // Every cell the selected ray intersects is occupied (the fixture is the
    // compact `Full` path fixture, not a materialized 256^3 volume).
    let fixture = Fixture::full(9);
    let origin = Vec3::new(0.5, 0.53, 0.53);
    let dir = Vec3::X;
    let hit = traverse(origin, dir, &fixture).expect("a fully occupied path must hit");
    assert_eq!(hit.material, 9);
    assert_eq!(hit.voxel, IVec3::new(128, 135, 135));
    assert!(
        hit.t.abs() <= EPS,
        "entry into the origin voxel is t=0, got {}",
        hit.t
    );
}

#[test]
fn nearest_hit_ordering_on_same_ray() {
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::new(130, 135, 135), 3);
    mip0.insert(IVec3::new(150, 135, 135), 5);
    let (fixture, mip0) = fixture_with(&mip0);
    let origin = Vec3::new(0.4, 0.53, 0.53);
    let dir = Vec3::X;

    let hit = traverse(origin, dir, &fixture).expect("ray must hit the near voxel");
    assert_eq!(hit.voxel, IVec3::new(130, 135, 135));
    assert_eq!(hit.material, 3);
    let expected_t = 130.0 / 256.0 - 0.4;
    assert!(
        (hit.t - expected_t).abs() <= EPS,
        "t={} expected ~{expected_t}",
        hit.t
    );

    assert_agree(&fixture, &mip0, origin, dir);
}

#[test]
fn entry_exactly_on_positive_voxel_boundary() {
    // Ray starts exactly on the boundary between voxels 129 and 130, moving
    // positive: the positive-side cell (130) must be selected.
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::new(130, 135, 135), 2);
    let (fixture, mip0) = fixture_with(&mip0);
    let origin = Vec3::new(130.0 / 256.0, 0.53, 0.53);
    let dir = Vec3::X;

    let hit = traverse(origin, dir, &fixture).expect("boundary entry must hit");
    assert_eq!(hit.voxel, IVec3::new(130, 135, 135));
    assert_eq!(hit.material, 2);
    assert!(
        hit.t.abs() <= EPS,
        "entry at the boundary is t=0, got {}",
        hit.t
    );

    assert_agree(&fixture, &mip0, origin, dir);
}

#[test]
fn entry_exactly_on_negative_voxel_boundary() {
    // Ray starts exactly on the boundary between voxels 129 and 130, moving
    // negative: the negative-side cell (129) must be selected (negative
    // boundary correction).
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::new(129, 135, 135), 2);
    let (fixture, mip0) = fixture_with(&mip0);
    let origin = Vec3::new(130.0 / 256.0, 0.53, 0.53);
    let dir = -Vec3::X;

    let hit = traverse(origin, dir, &fixture).expect("negative boundary entry must hit");
    assert_eq!(hit.voxel, IVec3::new(129, 135, 135));
    assert_eq!(hit.material, 2);
    assert!(
        hit.t.abs() <= EPS,
        "entry at the boundary is t=0, got {}",
        hit.t
    );

    assert_agree(&fixture, &mip0, origin, dir);
}

#[test]
fn multi_axis_corner_tie_advances_all_tied_axes() {
    // The +X/+Y diagonal crosses voxel corners at exactly tied boundaries on
    // both axes; only all-axis tie advancement reaches the occupied voxel.
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::new(130, 130, 135), 4);
    let (fixture, mip0) = fixture_with(&mip0);
    let origin = Vec3::new(0.5, 0.5, 135.5 / 256.0);
    let dir = Vec3::new(1.0, 1.0, 0.0).normalize();

    let hit = traverse(origin, dir, &fixture).expect("corner-tie ray must hit");
    assert_eq!(hit.voxel, IVec3::new(130, 130, 135));
    assert_eq!(hit.material, 4);
    let expected_t = (130.0 / 256.0 - 0.5) / dir.x;
    assert!(
        (hit.t - expected_t).abs() <= EPS,
        "t={} expected ~{expected_t}",
        hit.t
    );

    assert_agree(&fixture, &mip0, origin, dir);
}

#[test]
fn coordinate_mapping_low_and_high_mip0_coordinates() {
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::ZERO, 1);
    mip0.insert(IVec3::splat(255), 6);
    let (fixture, mip0) = fixture_with(&mip0);

    // Low corner: origin interior to voxel (0,0,0), moving +diagonal.
    let origin_lo = Vec3::new(0.0005, 0.0005, 0.0005);
    let dir_lo = Vec3::ONE.normalize();
    let hit = traverse(origin_lo, dir_lo, &fixture).expect("low-corner voxel must hit");
    assert_eq!(hit.voxel, IVec3::ZERO);
    assert_eq!(hit.material, 1);
    assert!(hit.t.abs() <= EPS);
    assert_agree(&fixture, &mip0, origin_lo, dir_lo);

    // High corner: origin interior to voxel (255,255,255), moving -diagonal.
    let origin_hi = Vec3::new(0.997, 0.997, 0.997);
    let dir_hi = -Vec3::ONE.normalize();
    let hit = traverse(origin_hi, dir_hi, &fixture).expect("high-corner voxel must hit");
    assert_eq!(hit.voxel, IVec3::splat(255));
    assert_eq!(hit.material, 6);
    assert!(hit.t.abs() <= EPS);
    assert_agree(&fixture, &mip0, origin_hi, dir_hi);
}

#[test]
fn generated_hierarchy_axis_aligned_rays_match_oracle() {
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::new(100, 100, 100), 2);
    mip0.insert(IVec3::new(100, 100, 105), 3);
    mip0.insert(IVec3::new(200, 200, 200), 5);
    mip0.insert(IVec3::new(30, 60, 90), 1);
    let (fixture, mip0) = fixture_with(&mip0);

    // +X through (100,100,100).
    let origin = Vec3::new(0.1, 100.5 / 256.0, 100.5 / 256.0);
    let hit = traverse(origin, Vec3::X, &fixture).expect("+X ray must hit");
    assert_eq!(hit.voxel, IVec3::new(100, 100, 100));
    assert_eq!(hit.material, 2);
    assert!((hit.t - (100.0 / 256.0 - 0.1)).abs() <= EPS);
    assert_agree(&fixture, &mip0, origin, Vec3::X);

    // -X entering the chunk from outside, through (200,200,200).
    let origin = Vec3::new(1.05, 200.5 / 256.0, 200.5 / 256.0);
    let hit = traverse(origin, -Vec3::X, &fixture).expect("-X ray must hit");
    assert_eq!(hit.voxel, IVec3::new(200, 200, 200));
    assert_eq!(hit.material, 5);
    assert!((hit.t - (1.05 - 201.0 / 256.0)).abs() <= EPS);
    assert_agree(&fixture, &mip0, origin, -Vec3::X);

    // +Z through the (100,100,100) / (100,100,105) column: nearest first.
    let origin = Vec3::new(100.5 / 256.0, 100.5 / 256.0, 0.1);
    let hit = traverse(origin, Vec3::Z, &fixture).expect("+Z ray must hit");
    assert_eq!(hit.voxel, IVec3::new(100, 100, 100));
    assert_eq!(hit.material, 2);
    assert!((hit.t - (100.0 / 256.0 - 0.1)).abs() <= EPS);
    assert_agree(&fixture, &mip0, origin, Vec3::Z);

    // -Y entering the chunk from outside, through (30,60,90).
    let origin = Vec3::new(30.5 / 256.0, 1.05, 90.5 / 256.0);
    let hit = traverse(origin, -Vec3::Y, &fixture).expect("-Y ray must hit");
    assert_eq!(hit.voxel, IVec3::new(30, 60, 90));
    assert_eq!(hit.material, 1);
    // Negative direction enters voxel (30,60,90) at the upper y boundary of
    // y voxel 60: y = 61/256.
    assert!((hit.t - (1.05 - 61.0 / 256.0)).abs() <= EPS);
    assert_agree(&fixture, &mip0, origin, -Vec3::Y);

    // Axis-aligned ray through empty space: both implementations miss.
    let origin = Vec3::new(0.2, 0.8, 0.2);
    assert_agree(&fixture, &mip0, origin, Vec3::X);
    assert_eq!(traverse(origin, Vec3::X, &fixture), None);
    assert_eq!(oracle(origin, Vec3::X, &mip0), None);
}

#[test]
fn generated_hierarchy_diagonal_rays_match_oracle() {
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::new(100, 100, 100), 2);
    mip0.insert(IVec3::new(100, 100, 105), 3);
    mip0.insert(IVec3::new(200, 200, 200), 5);
    mip0.insert(IVec3::new(30, 60, 90), 1);
    let (fixture, mip0) = fixture_with(&mip0);

    // +X/+Y/+Z diagonal from (0.6,0.6,0.6) enters voxel (200,200,200) through
    // its lower corner: entry is a corner tie on all three axes.
    let origin = Vec3::new(0.6, 0.6, 0.6);
    let dir = Vec3::ONE.normalize();
    let hit = traverse(origin, dir, &fixture).expect("diagonal corner-entry ray must hit");
    assert_eq!(hit.voxel, IVec3::new(200, 200, 200));
    assert_eq!(hit.material, 5);
    assert!(hit.t > 0.3, "entry well past the origin, got {}", hit.t);
    assert_agree(&fixture, &mip0, origin, dir);

    // Diagonal through voxel (100,100,105), entering via its z-boundary.
    let origin = Vec3::new(0.1, 0.1, 0.1);
    let dir = Vec3::new(0.29258, 0.29258, 0.31211).normalize();
    let hit = traverse(origin, dir, &fixture).expect("diagonal ray through the cluster must hit");
    assert_eq!(hit.voxel, IVec3::new(100, 100, 105));
    assert_eq!(hit.material, 3);
    assert_agree(&fixture, &mip0, origin, dir);

    // Diagonal through empty space: both implementations miss.
    let origin = Vec3::new(0.1, 0.9, 0.1);
    let dir = Vec3::new(1.0, 0.2, 0.4).normalize();
    assert_agree(&fixture, &mip0, origin, dir);
    assert_eq!(traverse(origin, dir, &fixture), None);
    assert_eq!(oracle(origin, dir, &mip0), None);
}

#[test]
fn false_positive_coarse_cell_without_descendant_is_a_miss() {
    // Explicit levels with an intentionally malformed coarse cell: mip-4 cell
    // (8,8,8) is occupied but has no mip-0 descendants. Coarse occupancy must
    // never be rendered as a color or an immediate hit.
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::new(144, 135, 135), 7);
    let mut levels = Fixture::generated(&mip0).into_sparse();
    levels[4].insert(IVec3::new(8, 8, 8), 3); // malformed false positive
    let fixture = Fixture::sparse(levels);

    // This ray crosses the false-positive mip-4 cell (8,8,8) and the real
    // mip-4 cell (9,8,8), but no mip-0 content on its path: a miss for both.
    let origin = Vec3::new(0.49, 0.51, 0.51);
    let dir = Vec3::X;
    assert_eq!(traverse(origin, dir, &fixture), None);
    assert_eq!(oracle(origin, dir, &mip0), None);
}

#[test]
fn sibling_recovery_after_empty_occupied_branch() {
    // Same fixture: the ray first enters the occupied-but-empty mip-4 branch
    // (8,8,8), descends it fully (no hit), resumes the already-advanced
    // parent, and finds the later front-to-back sibling (9,8,8), which has the
    // mip-0 descendant (144,135,135). The sibling's material is returned.
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::new(144, 135, 135), 7);
    let mut levels = Fixture::generated(&mip0).into_sparse();
    levels[4].insert(IVec3::new(8, 8, 8), 3); // malformed false positive
    let fixture = Fixture::sparse(levels);

    let origin = Vec3::new(0.49, 0.53, 0.53);
    let dir = Vec3::X;
    let hit = traverse(origin, dir, &fixture).expect("later sibling must provide the hit");
    assert_eq!(hit.voxel, IVec3::new(144, 135, 135));
    assert_eq!(hit.material, 7);
    let expected_t = 144.0 / 256.0 - 0.49;
    assert!(
        (hit.t - expected_t).abs() <= EPS,
        "t={} expected ~{expected_t}",
        hit.t
    );
    assert_agree(&fixture, &mip0, origin, dir);
}

#[test]
fn zero_length_ray_is_a_miss() {
    let mut mip0 = HashMap::new();
    mip0.insert(IVec3::new(130, 135, 135), 3);
    let (fixture, mip0) = fixture_with(&mip0);
    let origin = Vec3::new(0.5, 0.53, 0.53);
    let zero = Vec3::ZERO;
    assert_eq!(traverse(origin, zero, &fixture), None);
    assert_eq!(oracle(origin, zero, &mip0), None);
}

// Small helper so the malformed-level tests can recover the sparse map.
impl Fixture {
    fn into_sparse(self) -> Vec<HashMap<IVec3, u8>> {
        match self {
            Self::Sparse(levels) => levels,
            Self::Full(_) => panic!("Full fixture has no sparse levels"),
        }
    }
}
