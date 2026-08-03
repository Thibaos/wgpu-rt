# Plan 021: Refresh plan 015 — drift check, Current state, and Stage-5 notes after the 2026-08-03 batch

> **Executor instructions**: This plan is a **document-only refresh** of an
> existing plan. It changes no source code. Its job is to make
> `plans/015-chunk-rewrite-8x8x8-storage-buffers.md` executable again: the
> drift check it ships with fails against the current tree, so any dispatched
> executor would STOP on step 1. Run every verification command and confirm
> the expected result before moving on. If anything in "STOP conditions"
> occurs, stop and report — do not improvise. When done, update the status
> row for this plan in `plans/README.md` unless a reviewer dispatches you and
> told you they maintain the index.
>
> **Drift check (run first)**: `git status --short -- src/` — expected:
> **empty** (no uncommitted changes to source; untracked/modified files under
> `plans/` are expected and fine — this plan file itself is untracked). HEAD
> must be `bde0db4` or later (`git log --oneline -1`). Then run the Step 1
> greps and confirm they still **match** (i.e. the stale claims are still
> present in plan 015). On any mismatch, STOP and report.

## Status

- **Priority**: P1 (blocks dispatching the P0 plan 015)
- **Effort**: S
- **Risk**: LOW (documentation only; risk is stale-wording residue, which the
  greps in the Done criteria catch)
- **Depends on**: none, but see the ordering note below
- **Category**: docs
- **Planned at**: commit `bde0db4`, 2026-08-03
- **Issue**: none

## Why this matters

Plan 015 is the P0 rewrite (8³-chunk storage-buffer voxel pool, flat-DDA
ray-query primary). It was drafted at the same commit as the kickoff edits,
and its **drift check assumes that kickoff state was uncommitted**:

> `git status --short` — expected: HEAD `f014d3b`; the plan-015 kickoff edits
> present and uncommitted (Tree64 cleanup in `CONTEXT.md`, `docs/research-*.md`,
> `plans/README.md`; the `docs/adr/` series deleted; `Cargo.toml`/`Cargo.lock`
> with the `tree64` dep removed; untracked `plans/014-*.md` and this file), ...

Reality (verified when this plan was written): HEAD is `bde0db4` with those
edits **committed**, `tree64` already absent from both `Cargo.toml` and
`Cargo.lock`, and `docs/adr/` already deleted. Two further plans were drafted
in the same batch and change files plan 015's "Current state" describes:
plan 020 (world-clipping stopgap: grid-centered loading, `CHUNKS_Y=8`, drop
diagnostics, `split_chunks`) and plan 022 (heatmap wiring in both shaders,
`WGPU_RT_HEATMAP` env). **As of this plan's writing, 020 and 022 have NOT
landed** (both rows are TODO in `plans/README.md`; `src/world/chunk.rs` still
has `CHUNKS_Y: u32 = 1`, `src/world/mod.rs` has no `split_chunks`, and no
shader reads `viewport_and_heatmap.z`). Plan 021 must therefore be robust to
both states: the executor checks the live files and quotes what is actually
there, with explicit branches for "020/022 landed" vs "not landed". If 015 is
dispatched before this refresh, its executor stops at the drift check and
reports back — the plan is unexecutable as written.

## Current state

The stale claims in `plans/015-chunk-rewrite-8x8x8-storage-buffers.md`
(before this plan runs; line numbers verified against the live file):

- Lines 9-16: drift-check blockquote quoted above (expects HEAD `f014d3b`
  and uncommitted kickoff edits). The stale tokens are at line 10
  (`f014d3b`) and line 11 (`uncommitted`).
- Line 109: `- \`Cargo.toml:15\`: \`tree64 = { git = ... }\` — dead (no \`use
  tree64\` in \`src/\`)` — the plan's "Current state" still lists a Cargo.toml
  line that no longer exists. Factual state: `tree64` is gone from
  `Cargo.toml` and `Cargo.lock` entirely (0 matches in either file).
- Lines 86-88 "Current state" bullet for `src/world/chunk.rs`: claims
  `CHUNKS_X/Y/Z` = 8×1×8 fixed grid. Plan 020 changes this to 8×8×8 (if it
  landed; otherwise it is still 8×1×8 and the bullet is correct as written).
- Lines 89-91 "Current state" bullet for `src/world/mod.rs`: claims
  `World::into_chunks` fills the fixed grid. Plan 020 adds `split_chunks`
  with drop diagnostics (if it landed; otherwise `into_chunks` is unchanged).
- Line 216 (Stage 3): "**\`Cargo.toml\`**: remove the \`tree64\` dependency;
  regenerate \`Cargo.lock\`" — tree64 is already removed; lock regeneration is
  a no-op unless `cargo check` demands it.
- Line 258 (Stage 5): stretch targets (absolute thresholds — "bistro_sm < 6
  ms GPU @1080p, monu1 < 3 ms, avg cells/ray < 5"). Plan 020's fix means the
  sweep runs on **unclipped** scenes; 015 must note that the historical
  016/019 numbers were measured on clipped fragments (5-13 chunks) and are
  not a valid baseline.
- Plan 015's "Current state" has **no `loader.rs` bullet**; the centering
  claim lives in the Stage-1 design text at line 141 ("`center_world`
  currently anchors on the 256-grid center") and the Changes table at line
  267.

Repo conventions: plan files are plain Markdown; the plan index lives in
`plans/README.md`; statuses use TODO | IN PROGRESS | DONE | BLOCKED |
REJECTED. This refresh must not change 015's design decisions or stage
content — only its drift/state/verification prose.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Drift verify | `git status --short -- src/` + `git log --oneline -1` | empty source diff; HEAD ≥ `bde0db4` |
| Stale-claim greps | `sed -n '1,20p' plans/015-chunk-rewrite-8x8x8-storage-buffers.md \| grep -n "f014d3b\|uncommitted"` | **no match after Step 2** (the only remaining `f014d3b` is the legitimate line-23 "Depends on" reference) |
| Stale-claim greps | `grep -n "tree64 = { git" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` | no match after Step 2 |
| Stale-claim greps | `grep -n "8×1×8" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` | no match after Step 2 (replaced by the live values or left as-is if 020 has NOT landed — see Step 2.2) |
| State verification | `grep -n "pub const CHUNKS_Y" src/world/chunk.rs` | read the value; branch on it (Step 2.2) |
| State verification | `grep -n "split_chunks\|dropped" src/world/mod.rs` | presence/absence decides the Step 2.2 branch |
| State verification | `grep -n "viewport_and_heatmap\.z\|WGPU_RT_HEATMAP" assets/shaders/*.wgsl src/app.rs` | presence/absence decides the Step 2.2 heatmap branch |
| Final greps | `grep -n "unclipped" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` | at least one match after Step 2.4 |

## Scope

**In scope** (the only files you may modify):
- `plans/015-chunk-rewrite-8x8x8-storage-buffers.md`
- `plans/README.md` (status row + one reconcile note)
- `plans/021-refresh-plan-015.md` (this file: per the template, the executor
  updates this plan's status row in `plans/README.md`, not this file — the
  "Done criteria" here are the gate; do not edit this file itself)

**Out of scope** (do NOT touch):
- Any `src/`, `assets/`, `tests/` file — this is a documentation plan.
- Plan 015's design decisions, stage structure, decision record, or STOP
  conditions — refresh prose only. In particular, do NOT delete the
  `f014d3b` reference in 015's "Depends on" line (line 23) — it is a
  legitimate plan-019 commit pointer and stays.
- The other plan files (020/022 etc.).
- Creating new sections or bullets in 015 beyond the specified refresh (see
  the loader.rs note in Step 2.2 — if 020 has not landed, that note is
  skipped entirely).

## Git workflow

- Branch: `advisor/021-refresh-plan-015` (or match the operator's workflow).
- Commit style: `docs: refresh plan 015 drift check after 2026-08-03 batch`
  (match `git log --oneline`).
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Confirm the stale claims

Run the greps in "Commands you will need" (drift + stale-claim greps).
Confirm that `f014d3b` appears at 015 line 10 (inside the drift block) and
line 23 (Depends on), that `uncommitted` appears at line 11, that
`tree64 = { git` appears at line 109, and that `8×1×8` appears at line 87.
Also run the two state-verification greps and record which branch applies:
020 landed (CHUNKS_Y = 8, split_chunks present) or NOT landed (CHUNKS_Y = 1,
no split_chunks); 022 landed (shader reads viewport_and_heatmap.z,
WGPU_RT_HEATMAP present) or NOT landed.

If `f014d3b` no longer appears at 015 line 10 (someone already refreshed
the drift check), STOP and report — the plan's premise is gone.

**Verify**: `grep -n "f014d3b" plans/015-*.md` → at least the line-23 match,
plus the line-10 match before Step 2. `git status --short -- src/` → empty.

### Step 2: Rewrite the drift check and Current state

Edit `plans/015-chunk-rewrite-8x8x8-storage-buffers.md`:

1. Replace the **entire** drift-check blockquote (lines 9-16, from `>
   **Drift check (run first):**` through the closing `> STOP and report the
   exact changed path and difference.` line) with:
   ```markdown
   > **Drift check (run first)**:
   > `git status --short -- src/` — expected: empty (no uncommitted source
   > changes; untracked/modified files under `plans/` are expected and
   > fine). HEAD must be `bde0db4` or later.
   > The kickoff edits are committed: `docs/adr/` is deleted and `tree64` is
   > absent from `Cargo.toml`/`Cargo.lock`.
   > Plans 020 (grid-centered loading, CHUNKS_Y=8, drop diagnostics) and
   > 022 (heatmap wiring, WGPU_RT_HEATMAP) may or may not have landed —
   > verify every "Current state" excerpt below against the live files
   > before starting (grep CHUNKS_Y in src/world/chunk.rs, split_chunks in
   > src/world/mod.rs, viewport_and_heatmap.z in assets/shaders/). On any
   > mismatch, STOP and report the exact changed path and difference.
   ```
2. Update the "Current state" bullets (lines 86-91) to quote the live code,
   branching on the state you recorded in Step 1:
   - **`src/world/chunk.rs` bullet (lines 86-88)**:
     - If 020 landed: replace `8×1×8` with the live values — open
       `src/world/chunk.rs` and quote exactly what is there (expect
       `CHUNKS_Y: u32 = 8` and `CHUNKS_Y_INT: i32 = 8`).
     - If 020 has NOT landed: leave this bullet exactly as authored
       (8×1×8 is still true).
   - **`src/world/mod.rs` bullet (lines 89-91)**:
     - If 020 landed: reword the `World::into_chunks` clause to "fills the
       fixed grid via `split_chunks()` (counts + logs dropped voxels)" —
       quote the live signatures from `src/world/mod.rs`.
     - If 020 has NOT landed: leave the bullet as authored (`into_chunks`
       is unchanged).
   - **Tree64 bullet (line 109)**: replace with:
     ```markdown
     - `tree64` is fully removed (0 matches in `Cargo.toml` and
       `Cargo.lock`) — already handled at kickoff; Stage 3's dependency
       step is now a verify-absent step.
     ```
     (Note the leading `- ` — the replacement is a list bullet.)
   - **`assets/shaders/rayquery.wgsl` and `src/app.rs` bullets**: if 022
     landed, add one line to the relevant bullet noting the heatmap branch
     reads `camera.viewport_and_heatmap.z` and `WGPU_RT_HEATMAP=1` enables
     the heatmap at startup (env var read in `App::init`; it does not
     "default" anything) — and that Stage 2's flat-DDA port must preserve
     that read. If 022 has NOT landed, leave the bullets as authored.
   - **Loader/centering note**: plan 015 has no `loader.rs` bullet; the
     claim lives at line 141 (Stage-1 design: "`center_world` currently
     anchors on the 256-grid center"). If 020 landed, append
     " (post-020: anchors on the grid center)" to that sentence. If 020 has
     NOT landed, leave line 141 as authored. Do NOT create a new bullet.
3. In Stage 3 (the "drop tree64" bullet, line 216), rewrite:
   ```markdown
   - **`Cargo.toml`**: `tree64` is already removed (kickoff). Run
     `cargo check`; regenerate `Cargo.lock` only if cargo reports it dirty.
   ```
4. In Stage 5 (line 258, the stretch targets), insert one sentence before
   the targets:
   ```markdown
   > Note: if plan 020 landed, these scenes are **unclipped** (bistro_sm
   > now spans the full 2048³ world). The historical 016/019 numbers were
   > measured on clipped fragments (5-13 chunks) and are NOT a valid
   > baseline for this sweep — record fresh pre-rewrite numbers on the full
   > scenes instead. The targets below are absolute thresholds for the
   > unclipped scene, not regression gates against 016/019.
   ```
5. In 015's "Risks" section, add one line noting plans 020/022 changed the
   world/chunk layer and shaders after this plan was authored, so the
   "Current state" excerpts can drift — re-verify at dispatch (already
   covered by the rewritten drift check). Add this as a bullet in the
   existing Risks list only (015 has no separate dependency list; do not
   create one).

Do not touch any other content: decisions, stage steps (other than the
explicit edits above), STOP conditions, out-of-scope list all stay as
authored.

**Verify**:
- `sed -n '1,20p' plans/015-chunk-rewrite-8x8x8-storage-buffers.md | grep -n "f014d3b\|uncommitted"` → **no match**
- `grep -n "tree64 = { git" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` → **no match**
- `grep -n "8×1×8" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` → **no match** (if 020 landed) OR unchanged (if 020 has not landed — record which case you were in)
- `grep -n "unclipped" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` → **at least one match**
- `grep -n "already removed" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` → **at least one match**
- Re-read 015 lines 1-30 to confirm the drift check reads coherently (no duplicated closing sentence).

### Step 3: Update the index

In `plans/README.md`: set plan 021's status to DONE with a one-line summary,
and add a reconcile note that is accurate in EITHER state:
"015's drift check refreshed at HEAD `bde0db4`+; Current-state values
re-verified at dispatch (record whether 020/022 had landed when 021 ran:
TODO vs DONE). 015 remains TODO/P0 and is dispatchable."

**Verify**: `grep -n "021" plans/README.md` → the status row (the row whose
Plan column is `021`) reads DONE.

## Test plan

Documentation-only plan; the "tests" are the greps in Step 2's Verify and
the Done criteria. No test files exist for plan files; a human reviewer
should read plan 015's first 40 lines (drift check + Status + Current state)
and the Stage-3/Stage-5 regions to confirm the refresh reads coherently
end-to-end.

## Done criteria

ALL must hold (record the 020/022 branch you were in at the top of your
report):

- [ ] `git status --short -- src/` → empty; HEAD `bde0db4` or later
- [ ] `sed -n '1,20p' plans/015-chunk-rewrite-8x8x8-storage-buffers.md | grep -n "f014d3b\|uncommitted"` → no match (the line-23 `f014d3b` "Depends on" reference is INTENTIONALLY kept)
- [ ] `grep -n "tree64 = { git" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` → no match
- [ ] `grep -n "8×1×8" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` → no match if 020 landed; unchanged if not (state recorded)
- [ ] `grep -n "unclipped" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` → at least one match
- [ ] `grep -n "already removed" plans/015-chunk-rewrite-8x8x8-storage-buffers.md` → at least one match
- [ ] Every "Current state" bullet in plan 015 quotes the live file state (spot-check `src/world/chunk.rs` CHUNKS_Y, `src/world/mod.rs` into_chunks/split_chunks, `Cargo.toml` tree64 absence — per the branch you recorded)
- [ ] `plans/README.md` row 021 = DONE with summary
- [ ] No source file outside `plans/` is modified

## STOP conditions

Stop and report back (do not improvise) if:

- `f014d3b` no longer appears at 015 line 10 in Step 1 (premise already
  resolved — someone refreshed it first).
- A "Current state" bullet in plan 015 contradicts the live file in a way
  that can't be resolved by quoting the live code (e.g. the code was
  rewritten in a way that invalidates a whole stage — that would be a new
  plan's job, not this refresh's).
- The greps in Step 2's Verify fail twice after a reasonable fix attempt.
- You find yourself tempted to edit plan 015's decisions, stage steps
  beyond the explicit edits, or the line-23 "Depends on" reference — that
  is out of scope; report instead.

## Maintenance notes

- **When 015 executes**, its executor will re-run the drift check against
  whatever landed in the meantime; if the world/chunk layer changed again
  (e.g. more stopgaps), the executor should stop and this plan's refresh
  pattern applies again.
- **The unclipped-scenes note in Stage 5 changes the meaning of the stretch
  targets** (avg cells/ray < 5, bistro < 6 ms GPU): those targets were set
  against clipped measurements; whoever reviews 015's Stage-5 results should
  treat them as targets for the full scene, not as regression gates against
  plans 016/019.
- **Plans 020/022 were drafted in the same batch as this refresh but had
  NOT landed when 021 was written.** If you run this plan BEFORE 020 or 022,
  the Current-state bullets you write will be pre-020/022 — that is
  acceptable (the rewritten drift check re-verifies at dispatch), but record
  which state you wrote in the index note. The ideal order remains 020 → 022
  → 021.
