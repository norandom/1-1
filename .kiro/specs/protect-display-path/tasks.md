# Tasks: protect display_path.rs before it is reduced

One task per test. Each test is first shown failing on the planted fault and passing on the
unchanged code (`lesscode-core step pin`; the receipts are in `pins/`). All tests go into
`src-tauri/src/renderer/display_path/protect.rs`; `display_path.rs` is not changed.

- [x] 1. The platform is the one this build is for
  - Fixes: `Platform::current` names Windows in a Windows build, Linux in a Linux build, and
    "other" elsewhere.
  - Faults: `negate@68:11`, `negate@70:18`
  - _Boundary: protect.rs_
  - _Requirements: 1.1_
- [x] 2. A fallback moves the state to the canvas with its reason
  - Fixes: after `fall_back` the state names the canvas and the text of the reason given.
  - Faults: `drop_statement@448:8`
  - _Boundary: protect.rs_
  - _Requirements: 2.1_
- [x] 3. The layer asks for the backends of this platform
  - Fixes: DX12 on Windows; Vulkan and GL elsewhere.
  - Faults: `negate@497:7`
  - _Boundary: protect.rs_
  - _Requirements: 3.1_
- [x] 4. decide tries the GPU when nothing rules the layer out
  - Fixes: on Windows, with the native layer asked for, the answer is the native layer or "no
    usable GPU adapter", never "the GPU was not tried". The test asks the real GPU for an adapter.
  - Faults: `negate@486:7`, `drop_statement@487:8`
  - _Boundary: protect.rs_
  - _Requirements: 4.1_
- [x] 5. Proposals to write off, for the owner
  - `arg_literal:string->empty@481:47`, `drop_statement@458:4`, `drop_statement@461:4`, in
    `write-offs.json`, each with its reason.
  - _Requirements: 4.2, 5.1_

## Result, 2026-11-24

`lesscode-core step check`: accepted (`verdict.json`). Noticed before and now: `current` 0 of 2
and 2 of 2; `fall_back` 0 of 1 and 1 of 1; `layer_backends` 0 of 1 and 1 of 1; `decide` 0 of 3
and 2 of 3; `runtime_fallback` 0 of 2 and 0 of 2. The three faults left are written off with the
owner's approval (`write-offs.json`). All five functions are protected enough.
`display_path.rs` is unchanged since the base. The owner accepted the four tests. Known gap:
nothing checks that `runtime_fallback` moves the sessions to the canvas (line 461); a reduction of
that function has to keep the call by review.
