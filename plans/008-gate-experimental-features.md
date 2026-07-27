# Plan 008: Stop requesting all experimental GPU features

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

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: correctness/stability
- **Planned at**: commit `36b178e`, 2026-07-27

## Why this matters

`RenderContext::init` requests a device with
`experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() }`. That
`unsafe` block enables **every** experimental feature the adapter exposes,
unconditionally. Experimental features in wgpu are gated behind `unsafe`
precisely because they can be unstable, backend-specific, or unsound to combine.
Requesting all of them can force an unexpected code path, enable behavior the
shaders never rely on, or — on some drivers — trigger a software fallback.

None of the shaders in this repo (`assets/shaders/aabb_texture.wgsl`) use any
experimental feature. The features the app actually needs
(`TEXTURE_BINDING_ARRAY`, `SHADER_INT64`) are stable `wgpu::Features`, requested
via `App::required_features()` — they are **not** experimental. So the
`ExperimentalFeatures::enabled()` call is pure risk with zero benefit.

## Current state

`src/framework.rs`, inside `RenderContext::init` (the `request_device` call,
around lines 125–135):

```rust
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: (App::optional_features() & adapter.features())
                    | App::required_features(),
                required_limits: needed_limits,
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("Unable to find a suitable GPU adapter!");
```

`App::required_features()` (in `src/app.rs`) returns
`TEXTURE_BINDING_ARRAY | SHADER_INT64` — both stable `wgpu::Features`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Check   | `cargo check` | exit 0, no errors |
| Clippy  | `cargo clippy -- -D warnings` | exit 0, no warnings |
| Format  | `cargo fmt --check` | exit 0 |

## Scope

**In scope** (files you may modify):

- `src/framework.rs` — the single `experimental_features` line in the
  `request_device` call.
- `plans/README.md` — update this plan's status row.

**Out of scope** (do NOT touch):

- `src/app.rs` (`required_features` / `optional_features`).
- Any other field of the `DeviceDescriptor`.
- Anything else in `RenderContext::init`.

## Steps

### Step 1: Replace the unsafe experimental-features line

In `src/framework.rs`, inside the `request_device` call, change:

```rust
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
```

to:

```rust
                experimental_features: wgpu::ExperimentalFeatures::empty(),
```

This is the only change. It removes the `unsafe` block and requests **no**
experimental features, matching what the shaders actually use.

**Verify**: `cargo check` → exit 0. `cargo clippy -- -D warnings` → exit 0
(no `unsafe` block remains, so no unused-unsafe or safety lint should fire).
`cargo fmt --check` → exit 0.

### Step 2: Run the app to confirm it still acquires a device

Run the app briefly (a few seconds is enough) on the primary development
machine:

```
cargo run
```

**Expected**: the app launches, logs `Selected adapter: <name> (<backend>)` and
`Entering event loop...`, and shows the window. It must **not** panic with
"Unable to find a suitable GPU adapter!" — if it does, that means a feature the
app actually needs was accidentally being supplied only via the experimental
path; treat that as a STOP condition (see below) and report back.

Close the window with Escape.

**Verify**: app launches without a panic. (No automated assertion; this is a
smoke check.)

## STOP conditions

- If after the change `cargo run` panics at device acquisition with a missing-
  feature error, **stop**. That would mean some required feature was
  previously satisfied only through `ExperimentalFeatures::enabled()` rather
  than through `App::required_features()`. Do not re-add the blanket
  `enabled()`; instead report which feature is missing so it can be added
  explicitly to `App::required_features()`.
- If `git diff --stat 36b178e..HEAD -- src/framework.rs` shows the file has
  changed and the `experimental_features` line is no longer present or no
  longer matches the excerpt above, **stop** and report the drift before
  editing.

## Machine-checkable done criteria

- `grep -n "ExperimentalFeatures::enabled" src/framework.rs` → no matches.
- `grep -n "unsafe" src/framework.rs` → no matches in `RenderContext::init`.
- `cargo clippy -- -D warnings` → exit 0.
- `cargo fmt --check` → exit 0.
- `cargo run` launches the window without panicking at device acquisition.

## Test plan

No new tests. This is a one-line change to GPU device-request flags with no
deterministic CPU logic to unit-test; the smoke check in Step 2 is the
verification.

## Maintenance note

If a future change adds a shader that genuinely requires an experimental
wgpu feature, that feature must be requested **explicitly** by name (e.g.
`unsafe { wgpu::ExperimentalFeatures::from_bits_truncate(...) }` with a comment
explaining why), never via the blanket `enabled()`. Watch for this in review of
any `experimental_features:` line.
