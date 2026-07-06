# Plan 002: Handle surface acquire timeout and outdated gracefully

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat d31e828..HEAD -- src/framework.rs`
> If `src/framework.rs` changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `d31e828`, 2026-07-06

## Why this matters

`SurfaceWrapper::acquire` only handles `CurrentSurfaceTexture::Success`,
panicking on `Timeout` and `OutdatedOutdated`. A `Timeout` occurs when the GPU
is busy (multiple frames in flight, driver stall) — the app should retry
instead of crashing. An `OutdatedOutdated` occurs after window resize, sleep/resume,
or driver reset — the surface should be reconfigured and retried. The panic
makes the app fragile under normal OS/driver conditions.

## Current state

- `src/framework.rs:46-53` — `SurfaceWrapper::acquire`:

```rust
fn acquire(&mut self, context: &RenderContext) -> wgpu::SurfaceTexture {
    let surface = self.surface.as_ref().unwrap();

    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame) => frame,
        _ => {
            surface.configure(&context.device, self.config());
            panic!("Failed to acquire next surface texture!");
        }
    }
}
```

The `_` arm already reconfigures the surface (good instinct — `OutdatedOutdated`
requires reconfiguration), but then panics instead of retrying.

- `src/framework.rs:284-292` — caller in `RedrawRequested`:

```rust
WindowEvent::RedrawRequested => {
    self.frame_counter.update();

    let frame = self.surface.acquire(context);
    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(self.surface.config().view_formats[0]),
        ..Default::default()
    });

    app.render(&view, &context.device, &context.queue, &self.pressed_keys);
    // ...
    window.request_redraw();
}
```

If `acquire` fails, `request_redraw()` is never called, so the app stops
rendering even after a graceful recovery.

## Commands you will need

| Purpose  | Command                       | Expected on success   |
|----------|-------------------------------|-----------------------|
| Build    | `cargo build`                 | exit 0, "Finished"    |
| Format   | `cargo fmt --check`           | exit 0, no diff       |
| Lint     | `cargo clippy -- -D warnings` | exit 0, no warnings   |
| Test     | `cargo test`                  | exit 0, 0 tests run   |

## Scope

**In scope** (the only files you should modify):
- `src/framework.rs`

**Out of scope** (do NOT touch):
- `src/app.rs` — shape of `App::render()` unchanged.
- `src/utils.rs` — no surface/texture logic there.
- Any change to the `RedrawRequested` handler's rendering order.

## Git workflow

- Branch: `advisor/002-surface-acquire-panic`
- Commit per step; message style: `fix: <description>`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Replace `SurfaceWrapper::acquire` with retry logic

Replace the entire method body with a loop that handles all three variants:

- **Success**: return the frame.
- **Timeout**: retry up to 3 times, logging a warning each time. If all retries
  exhausted, reconfigure and try once more. If still failing, continue to the
  fallback (Step 2).
- **OutdatedOutdated**: reconfigure the surface once, then retry acquire.
  On second failure, treat as unrecoverable (fallback).

The method signature changes: it now returns `Option<wgpu::SurfaceTexture>`
instead of panicking. The caller handles `None` by skipping the frame.

New implementation:

```rust
fn acquire(&mut self, context: &RenderContext) -> Option<wgpu::SurfaceTexture> {
    let surface = self.surface.as_ref().unwrap();
    let config = self.config();

    // Try up to 3 times for transient Timeout
    for attempt in 0..3 {
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => return Some(frame),
            wgpu::CurrentSurfaceTexture::Timeout => {
                log::warn!(
                    "Surface acquire timed out (attempt {}/3), retrying...",
                    attempt + 1
                );
                // Brief yield to let the GPU pipeline drain
                std::thread::yield_now();
            }
            wgpu::CurrentSurfaceTexture::OutdatedOutdated => {
                log::info!("Surface outdated, reconfiguring...");
                surface.configure(&context.device, config);
                // After reconfiguration, retry once
                break;
            }
        }
    }

    // Final attempt: reconfigure and try one more time
    surface.configure(&context.device, config);
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
        other => {
            log::error!("Failed to acquire surface texture after reconfiguration: {other:?}");
            None
        }
    }
}
```

The `wgpu::CurrentSurfaceTexture` enum does not derive `Debug`, so `{other:?}`
in the log line won't compile. Use a simpler message instead:

```rust
log::error!("Failed to acquire surface texture after reconfiguration");
```

**Verify**: `cargo build` → exit 0. If the `std::thread::yield_now()` import
is missing, add `use std::thread;` at the top of the file. (Actually it's
`std::thread::yield_now()` which doesn't need an import — it's a free function.)

### Step 2: Update caller to handle `Option`

In `WindowEvent::RedrawRequested`, wrap the acquire in an `if let`:

Replace:

```rust
WindowEvent::RedrawRequested => {
    self.frame_counter.update();

    let frame = self.surface.acquire(context);
    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(self.surface.config().view_formats[0]),
        ..Default::default()
    });

    app.render(&view, &context.device, &context.queue, &self.pressed_keys);

    window.pre_present_notify();
    context.queue.present(frame);

    window.request_redraw();
}
```

With:

```rust
WindowEvent::RedrawRequested => {
    self.frame_counter.update();

    if let Some(frame) = self.surface.acquire(context) {
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.surface.config().view_formats[0]),
            ..Default::default()
        });

        app.render(&view, &context.device, &context.queue, &self.pressed_keys);

        window.pre_present_notify();
        context.queue.present(frame);
    }
    // Always request redraw — even on acquire failure, we want to keep
    // trying on the next frame.
    window.request_redraw();
}
```

Note: `window.request_redraw()` is moved OUTSIDE the `if let` block so the
event loop keeps ticking even when acquire fails. This prevents a permanent
freeze if the surface stays in a bad state.

**Verify**: `cargo build` → exit 0, "Finished"

### Step 3: Final check

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo build
cargo test
```

All must exit 0.

## Test plan

- No new automated tests (surface acquire is a GPU I/O operation).
- Manual verification: the app should run normally. Hard to deliberately
  trigger Timeout/OutdatedOutdated without external interference, but the normal
  path (Success) must still work.
- Sleep/resume the system while the app is running — previously this could
  trigger an OutdatedOutdated state; after the fix, the app should recover.

## Done criteria

- [ ] `cargo build` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -- -D warnings` exits 0
- [ ] `cargo test` exits 0
- [ ] `grep "panic.*acquire\|panic.*surface texture" src/framework.rs` returns no matches
- [ ] `grep "fn acquire" src/framework.rs` shows return type `Option<wgpu::SurfaceTexture>`
- [ ] `grep "request_redraw" src/framework.rs` — the call in `RedrawRequested` is OUTSIDE the `if let` block (not inside it)
- [ ] No files outside `src/framework.rs` modified
- [ ] `plans/README.md` status row updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:

- The code at the locations in "Current state" doesn't match the excerpts
  (the codebase has drifted since this plan was written).
- `cargo build` fails with compile errors that can't be resolved by adjusting
  the `match` arms (e.g., if `CurrentSurfaceTexture` variants differ from
  what's documented here).
- The `#[derive(Debug)]` addition to the log line doesn't work (wgpu may not
  derive Debug for `CurrentSurfaceTexture` in all versions). Fall back to a
  non-specific log message.
- You discover the `surface.configure()` call inside the retry loop causes
  a double-borrow of `self` — the surface and config borrows overlap.

## Maintenance notes

- The 3-retry policy for Timeout is heuristic. If frame times are very high
  (render time > present interval), Timeout may fire more often. Consider
  increasing retries or adding a small sleep between attempts.
- The `OutdatedOutdated`→reconfigure→retry pattern follows the standard wgpu
  surface lifecycle. Future wgpu versions may rename or add variants; keep
  the match exhaustive.
- The `request_redraw()` outside the `if let` means the event loop keeps busy
  even on persistent failure. This is intentional for recovery but could
  spin the CPU if the surface is permanently broken. A future improvement
  could add a failure counter and backoff.
