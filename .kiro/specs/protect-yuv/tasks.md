# Tasks: protect yuv.rs before it is reduced

One task per test. Each test is first shown failing on the planted fault and passing on the
unchanged code (`lesscode-core step pin`; the receipts are in `pins/`). All tests go into
`src-tauri/src/renderer/yuv/protect.rs`; `src-tauri/src/renderer/yuv.rs` is not changed.

- [x] 1. An NV12 frame converts on the GPU as the reference does
  - Fixes: an NV12 frame (Y plane, then U and V interleaved in one plane) converted by
    `YuvConverter` on the GPU gives the reference picture of the same frame, within one step of
    rounding, for both matrices; without a GPU adapter the test says so and passes.
  - Faults: `drop_statement@114:8` (#5)
  - Pins: `pins/pin-5.json` (admitted 2026-11-24, 11 min 21 s; the log is kept outside the repository, which ignores `*.log`)
  - _Boundary: src-tauri/src/renderer/yuv/protect.rs; test files only_
  - _Requirements: 3.1_

- [x] 2. Proposals to write off, for the owner of the code
  - `arg_literal:string->empty@296:12`, `@305:20`, `@312:20`, `@322:20`, `@333:24`, `@358:28`
    (`convert`) and `arg_literal:string->empty@174:24`, `@188:24`, `@206:24`, `@211:24`,
    `@236:24` (`new`), in `write-offs.json`, each with its kind and reason.
  - _Requirements: 1.1, 2.1_

## Result, 2026-11-24

Not judged yet.
