# Tasks: protect shared_frame.rs before it is reduced

One task per test. Each test is first shown failing on the planted fault and passing on the
unchanged code (`lesscode-core step pin`; the receipts are in `pins/`). All tests go into
`src-tauri/src/renderer/shared_frame/protect.rs`; `src-tauri/src/renderer/shared_frame.rs` is not
changed.

- [ ] 1. An empty rectangle leaves no damage
  - Fixes: a rectangle with no width or no height, written through the write guard, leaves
    `take_damage` with nothing to return, while a 1x1 rectangle leaves damage at its place.
  - Faults: `swap:<-><=@93:21`, `swap:&&->||@93:33`
  - Pins: `pins/pin-<n>.json` per fault
  - _Boundary: src-tauri/src/renderer/shared_frame/protect.rs; test files only_
  - _Requirements: 1.1_

- [ ] 2. Data too short for a new size leaves the frame as it was
  - Fixes: `update_full` with data shorter than the new size returns `false` and keeps the frame's
    size, pixels, damage and version, through the guard and through the frame.
  - Faults: `swap_next@127:8`
  - Pins: `pins/pin-<n>.json`
  - _Boundary: src-tauri/src/renderer/shared_frame/protect.rs; test files only_
  - _Requirements: 2.1_

- [ ] 3. A single rectangle update marks the frame dirty
  - Fixes: one `SharedFrame::update_rect` raises the version by exactly one and makes the next
    `publish` return the new picture, once.
  - Faults: `drop_statement@211:8`
  - Pins: `pins/pin-<n>.json`
  - _Boundary: src-tauri/src/renderer/shared_frame/protect.rs; test files only_
  - _Requirements: 1.2_

- [ ] 4. Proposals to write off, for the owner of the code
  - `drop_statement@210:8`, `arg_literal:0->1@133:50`, `drop_statement@199:8`, in
    `write-offs.json`, each with its kind and reason.
  - _Requirements: 1.3, 2.2, 2.3_

## Result, 2026-11-24

Not judged yet.
