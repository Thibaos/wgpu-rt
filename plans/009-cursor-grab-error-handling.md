# Plan 009: Handle cursor-grab errors instead of panicking

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 36b178e..HEAD -- src/framework.rs`
> If `src/framework.rs` changed since this plan was written, compare the
> "Current state" excerpt below against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: correctness/robustness
- **Planned at**: commit `36b178e`, 2026-07-27

## Why this matters

Pressing Escape toggles mouse capture between `CursorGrabMode::Confined` and
`CursorGrabMode::None`. Both calls use `.unwrap()`:

```rust
window.set_cursor_grab(CursorGrabMode::Confined).unwrap();
...
window.set_cursor_grab(CursorGrabMode::None).unwrap();
```

`set_cursor_grab` returns `Result<(), ExternalError>`. It fails on platforms or
window managers that do not support the requested mode — for example, some
Wayland compositors reject `Confined`, and `Locked` is preferred there. A failed
grab currently **panics the whole app** on Escape, which is a poor experience
for a debug toggle. The fix is to log the error and fall back gracefully.

## Current state

`src/framework.rs`, inside `Framework::window_event`, the `KeyboardInput` /
`ElementState::Pressed` arm for `Escape` (around lines 288–305):

```rust
                if let Key::Named(NamedKey::Escape) = &logical_key
                    && !self.pressed_keys.contains(&Key::Named(NamedKey::Escape))
                {
                    match self.cursor_grab_mode {
                        CursorGrabMode::None => {
                            self.cursor_grab_mode = CursorGrabMode::Confined;
                            window.set_cursor_grab(CursorGrabMode::Confined).unwrap();
                            window.set_cursor_visible(false);
                        }
                        _ => {
                            self.cursor_grab_mode = CursorGrabMode::None;
                            window.set_cursor_grab(CursorGrabMode::None).unwrap();
                            window.set_cursor_visible(true);
                        }
                    }
                }
```

`self.cursor_grab_mode` is a `CursorGrabMode` field on `Framework` (declared
near the top of the `Framework` struct definition).

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Check   | `cargo check` | exit 0, no errors |
| Clippy  | `cargo clippy -- -D warnings` | exit 0, no warnings |
| Format  | `cargo fmt --check` | exit 0 |

## Scope

**In scope** (files you may modify):

- `src/framework.rs` — the Escape-grab `match` arm only.
- `plans/README.md` — update this plan's status row.

**Out of scope** (do NOT touch):

- The `DeviceEvent::MouseMotion` handler (it already gates on
  `cursor_grab_mode == Confined`).
- The `cursor_grab_mode` field declaration or its initial value.
- Any other event handling.

## Design

Replace each `.unwrap()` with a small helper that attempts the requested mode
and, on failure, logs a warning and tries `CursorGrabMode::Locked` as a
fallback (the mode more widely supported on Wayland). If the fallback also
fails, log and leave the grab as-is. The tracked `self.cursor_grab_mode` must
reflect the mode that actually succeeded (so the `MouseMotion` gate and the
next Escape toggle stay consistent).

Introduce a private helper method on `Framework`:

```rust
    /// Attempt a cursor-grab mode, falling back to `Locked`, then to no grab.
    /// Returns the mode that actually took effect (or `None` if all failed).
    fn try_set_cursor_grab(
        &self,
        window: &Window,
        desired: CursorGrabMode,
    ) -> CursorGrabMode {
        match window.set_cursor_grab(desired) {
            Ok(()) => desired,
            Err(e) => {
                log::warn!("set_cursor_grab({desired:?}) failed: {e}; trying Locked");
                match window.set_cursor_grab(CursorGrabMode::Locked) {
                    Ok(()) => CursorGrabMode::Locked,
                    Err(e2) => {
                        log::warn!("set_cursor_grab(Locked) also failed: {e2}; grab disabled");
                        CursorGrabMode::None
                    }
                }
            }
        }
    }
```

## Steps

### Step 1: Add the helper method

Add the `try_set_cursor_grab` method (above) to the `impl Framework` block in
`src/framework.rs`. Place it near the other `Framework` methods (e.g. right
after `fn new()`). It takes `&self` (it only reads `self` and uses `window`),
so it needs no `&mut self`.

**Verify**: `cargo check` → exit 0.

### Step 2: Use the helper in the Escape handler

Replace the Escape `match` arm shown in "Current state" with:

```rust
                if let Key::Named(NamedKey::Escape) = &logical_key
                    && !self.pressed_keys.contains(&Key::Named(NamedKey::Escape))
                {
                    let desired = match self.cursor_grab_mode {
                        CursorGrabMode::None => CursorGrabMode::Confined,
                        _ => CursorGrabMode::None,
                    };
                    let applied = self.try_set_cursor_grab(window, desired);
                    self.cursor_grab_mode = applied;
                    window.set_cursor_visible(self.cursor_grab_mode == CursorGrabMode::None);
                }
```

Notes for the executor:
- When `desired` is `None`, the helper will succeed immediately (releasing the
  grab always works) and return `None`, so `cursor_visible` becomes `true`.
- When `desired` is `Confined` and the platform rejects it, the helper tries
  `Locked`; if that also fails it returns `None` and the cursor stays visible.
  `cursor_visible` is then `true`, which is correct (no grab → show cursor).
- The `MouseMotion` handler already gates on `cursor_grab_mode == Confined`.
  If the platform only gives us `Locked`, mouse-look will not engage. That is
  an acceptable, non-crashing degradation for this plan; widening the gate to
  also accept `Locked` is a follow-up (see Maintenance note).

**Verify**: `cargo check` → exit 0. `cargo clippy -- -D warnings` → exit 0.
`cargo fmt --check` → exit 0.

### Step 3: Smoke test

```
cargo run
```

Press Escape once (cursor should hide / grab), press Escape again (cursor
should reappear). Repeat a few times. The app must **not** crash on any
platform. On a platform where `Confined` is unsupported, expect a
`set_cursor_grab(Confined) failed: ...; trying Locked` warning in the log and
the cursor hiding if `Locked` succeeds.

**Verify**: app survives repeated Escape presses without panicking.

## STOP conditions

- If `git diff --stat 36b178e..HEAD -- src/framework.rs` shows the Escape
  handler has been restructured and no longer matches the "Current state"
  excerpt, **stop** and report the drift before editing.
- If `set_cursor_grab`'s signature in wgpu 30 does not match
  `Result<(), ExternalError>` used here (check the compile error from Step 1),
  **stop** and report — do not change the wgpu version or add `as any`-style
  casts; report the actual signature so the helper can be adjusted.

## Machine-checkable done criteria

- `grep -n "set_cursor_grab" src/framework.rs | grep unwrap` → no matches
  (no `.unwrap()` on `set_cursor_grab`).
- `grep -n "try_set_cursor_grab" src/framework.rs` → at least two matches
  (definition + call site).
- `cargo clippy -- -D warnings` → exit 0.
- `cargo fmt --check` → exit 0.
- `cargo run` survives repeated Escape presses without panicking.

## Test plan

No new automated tests. The error path depends on the platform/WM and cannot be
unit-tested deterministically without mocking `winit::Window`. The smoke test
in Step 3 is the verification.

## Maintenance note

The `DeviceEvent::MouseMotion` arm in `window_event` currently gates mouse-look
on `cursor_grab_mode == CursorGrabMode::Confined`. If you later widen the
fallback so `Locked` is a first-class grab mode, update that gate to
`matches!(self.cursor_grab_mode, CursorGrabMode::Confined | CursorGrabMode::Locked)`
so mouse-look works under `Locked` too. Watch for this when reviewing any
change to cursor handling.
