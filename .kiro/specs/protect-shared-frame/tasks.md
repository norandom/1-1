# Tasks: protect shared_frame.rs before it is reduced

One task per test. Each test is first shown failing on the planted fault and passing on the
unchanged code (`lesscode-core step pin`; the receipts are in `pins/`). All tests go into
`src-tauri/src/renderer/shared_frame/protect.rs`; `src-tauri/src/renderer/shared_frame.rs` is not
changed.

- [x] 1. An empty rectangle leaves no damage
  - Fixes: a rectangle with no width or no height, written through the write guard, leaves
    `take_damage` with nothing to return, while a 1x1 rectangle leaves damage at its place.
  - Faults: `swap:<-><=@93:21` (#2), `swap:&&->||@93:33` (#3)
  - Pins: `pins/pin-2.json` (admitted, 9 min 57 s), `pins/pin-3.json` (admitted, 94 s) (the logs are kept outside the repository, which ignores `*.log`: `D:\BuildArtifacts\lesscode\brownfield-test-stage\stage-6\sf-pins\`)
  - _Boundary: src-tauri/src/renderer/shared_frame/protect.rs; test files only_
  - _Requirements: 1.1_

- [x] 2. Data too short for a new size leaves the frame as it was
  - Fixes: `update_full` with data shorter than the new size returns `false` and keeps the frame's
    size, pixels, damage and version, through the guard and through the frame.
  - Faults: `swap_next@127:8` (#11)
  - Pins: `pins/pin-11.json` (admitted, 5 min 35 s)
  - _Boundary: src-tauri/src/renderer/shared_frame/protect.rs; test files only_
  - _Requirements: 2.1_

- [x] 3. A single rectangle update marks the frame dirty
  - Fixes: one `SharedFrame::update_rect` raises the version by exactly one and makes the next
    `publish` return the new picture, once.
  - Faults: `drop_statement@211:8` (#30)
  - Pins: `pins/pin-30.json` (admitted, 5 min 26 s)
  - _Boundary: src-tauri/src/renderer/shared_frame/protect.rs; test files only_
  - _Requirements: 1.2_

- [x] 4. Proposals to write off, for the owner of the code
  - `drop_statement@210:8` (#29), `arg_literal:0->1@133:50` (#21), `drop_statement@199:8` (#25), in
    `write-offs.json`, each with its kind and reason.
  - Not proposed: `swap:>=->>@100:18` (#6). The plan counted it noticed, the base run unnoticed.
    Planted by hand, all 536 tests pass, so the plan's kill was false, most likely the drift-rdp
    GPU test that aborts when two copies run at once. It is equivalent: the bounds check below it
    skips the row. `stage.json` comes from the plan and does not have it, and `write-offs.json`
    refuses a fault the stage does not have, so it cannot be proposed here.
  - _Requirements: 1.3, 2.2, 2.3_

## Result, 2026-11-25

`lesscode-core step check`: accepted (`verdict.json`, `1d381f1`, the second check, with the base
run's unnoticed faults). `lesscode-core step protect result`: exit 0 (`protection.json`).

| Function | Noticed before | Noticed now | Written off | Waiting |
| --- | --- | --- | --- | --- |
| `update_rect` | 8 of 13 | 11 of 13 | 2 | 0 |
| `update_full` | 14 of 17 | 15 of 17 | 2 | 0 |

Written off, with the owner's approval (`write-offs.json`), all `nothing_a_caller_sees`:

- `drop_statement@210:8` and `drop_statement@199:8`: without `drop(guard)` the guard is released
  at the end of the same function, right after `mark_dirty`;
- `arg_literal:0->1@133:50`: every byte the resize adds is overwritten on the next line;
- `swap:>=->>@100:18`: taken into the stage from the base run (the plan counted it noticed). The
  bounds check below it skips the row.

The first three were approved by Marius Ciepluch on 2026-11-25 (`0b5fe69`), and the fourth the
same day (`39e09d4`).

All 2 functions are protected enough. `src-tauri/src/renderer/shared_frame.rs` is unchanged since
the base.

Known gap: nothing checks that `SharedFrame::update_rect` and `update_full` release the write lock
before they wake the render thread. A reduction of them has to keep `drop(guard)` before
`mark_dirty` by review.
