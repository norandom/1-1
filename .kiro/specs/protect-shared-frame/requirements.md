# Requirements: protect shared_frame.rs before it is reduced

Written 2026-11-24 from a plan of lesscode (`step plan`, 42 planted faults in
`src-tauri/src/renderer/shared_frame.rs`, 35 noticed).
Plan: `D:/BuildArtifacts/lesscode/brownfield-test-stage/stage-6/sf-plan.json`. Base: `a8f34429c8e4`.
Tests: `cargo nextest run --locked --manifest-path src-tauri/Cargo.toml --lib --no-fail-fast`.
Not approved yet: the owner of the code reviews once, at the end.

A planted fault is one small mistake made on purpose in a copy of the code. Where every test still
passes with the mistake in place, nothing would tell us if a later change made the same mistake.
The stage protects every function of the plan with an unnoticed fault: 2 functions. Each
requirement says what could change unseen there today.

The plan names functions by name only. `update_rect` stands for both `WriteGuard::update_rect`
and `SharedFrame::update_rect`, and `update_full` for both `WriteGuard::update_full` and
`SharedFrame::update_full`.

Protected already: `mark_dirty` (5 of 5 noticed), `copy_region`, `take_damage`, `take_full`,
`new`, `publish`.

The tests of the stage are in `src-tauri/src/renderer/shared_frame/protect.rs`, joined by
`#[cfg(test)] mod protect;` in `src-tauri/src/renderer/shared_frame.rs` (commit `a8f3442`, before
the plan).

## Requirement 1: an empty rectangle and a single rectangle (`update_rect`)

Today nothing notices when a rectangle with no width or no height is still recorded as damage for
the render thread to upload (faults `swap:<-><=@93:21`, `swap:&&->||@93:33`), or when a single
rectangle written through the frame itself is not announced: the version stays and the picture is
not published (fault `drop_statement@211:8`). Nor does anything notice when the write lock is held
a moment longer, until the end of `SharedFrame::update_rect` (fault `drop_statement@210:8`).

#### Acceptance Criteria
1. When a rectangle with no width or no height is written, the project's tests shall fail if it is
   recorded as damage.
2. When one rectangle is written through `SharedFrame::update_rect`, the project's tests shall fail
   if the version does not go up by one or the picture is not published.
3. The fault at 210:8 changes nothing a caller can see: without `drop(guard)` the guard is
   released at the end of the same function, after `mark_dirty`, and a woken render thread waits
   only that moment for the lock. It is proposed for writing off.

## Requirement 2: data too short for a new size (`update_full`)

Today nothing notices when data too short for a new size still resizes the frame before it is
rejected, against the promise that it "leaves the frame untouched" (fault `swap_next@127:8`). Nor
does anything notice when the bytes a resize adds start as 1 instead of 0
(`arg_literal:0->1@133:50`), or when the write lock is held until the end of
`SharedFrame::update_full` (`drop_statement@199:8`).

#### Acceptance Criteria
1. When `update_full` is given data shorter than its new size, the project's tests shall fail if
   the frame's size, pixels, damage or version change.
2. The fault at 133:50 changes nothing a caller can see: `write_buf` is resized to exactly
   `expected` bytes and every one of them is overwritten from `data` on the next line. It is
   proposed for writing off.
3. The fault at 199:8 changes nothing a caller can see, as 1.3: the guard is released at the end
   of the same function. It is proposed for writing off.
