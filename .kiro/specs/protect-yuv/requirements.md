# Requirements: protect yuv.rs before it is reduced

Written 2026-11-24 from a plan of lesscode (`step plan`, 30 planted faults in
`src-tauri/src/renderer/yuv.rs`, 18 noticed). Plan:
`D:/BuildArtifacts/lesscode/brownfield-test-stage/yuv/plan.json`. Base: `4fff14f`. Tests:
`cargo nextest run --locked --manifest-path src-tauri/Cargo.toml --lib --no-fail-fast`.
Not approved yet: the owner of the code reviews once, at the end.

A planted fault is one small mistake made on purpose in a copy of the code. Where every test still
passes with the mistake in place, nothing would tell us if a later change made the same mistake.
The owner chose the three functions of the file that have unnoticed faults, which is all of
`yuv.rs` that a reduction could change unseen. Each requirement says what could change unseen there
today.

Protected already: `reference_rgba` (2 of 2 noticed).

No fault planted: `chroma_size`, `coefficients`, `sample`. The plan says nothing about them.

The tests of the stage are in `src-tauri/src/renderer/yuv/protect.rs`, joined by
`#[cfg(test)] mod protect;` in `src-tauri/src/renderer/yuv.rs` (commit `4fff14f`, before the plan).

## Requirement 1: the names of the conversion's GPU objects (`convert`)

Today nothing notices when the textures of the planes, the bind group or the render pass of a
conversion are created without their debug names (faults `arg_literal:string->empty@296:12`,
`arg_literal:string->empty@305:20`, `arg_literal:string->empty@312:20`,
`arg_literal:string->empty@322:20`, `arg_literal:string->empty@333:24`,
`arg_literal:string->empty@358:28`).

#### Acceptance Criteria
1. The faults at 296:12, 305:20, 312:20, 322:20, 333:24 and 358:28 change nothing a caller can see:
   each blanks a wgpu debug label (`"yuv-y"`, `"yuv-u"`, `"yuv-v"`, `"yuv-uv"`, `"yuv-convert"`),
   which graphics debuggers and wgpu's own validation messages show, and no pixel and no return
   value depends on it. They are proposed for writing off.

## Requirement 2: the names of the converter's GPU objects (`new`)

Today nothing notices when the shader module, the bind group layout, the pipeline layout, the
pipeline or the uniform buffer are created without their debug name (faults
`arg_literal:string->empty@174:24`, `arg_literal:string->empty@188:24`,
`arg_literal:string->empty@206:24`, `arg_literal:string->empty@211:24`,
`arg_literal:string->empty@236:24`).

#### Acceptance Criteria
1. The faults at 174:24, 188:24, 206:24, 211:24 and 236:24 change nothing a caller can see: each
   blanks the wgpu debug label `"yuv-convert"`, which graphics debuggers and wgpu's own validation
   messages show, and no pixel and no return value depends on it. They are proposed for writing
   off.

## Requirement 3: an NV12 frame says that its chroma is interleaved (`uniform_bytes`)

Today nothing notices when the uniforms of an NV12 frame do not tell the shader that U and V are
interleaved in one texture (fault `drop_statement@114:8`). The shader then reads U and V as two
planes, and an NV12 picture comes out in wrong colours. The test that compares the GPU with the
reference converts planar frames only.

#### Acceptance Criteria
1. Where a GPU adapter is present, the project's tests shall fail when an NV12 frame converted by
   `YuvConverter` on the GPU differs from the reference picture of the same frame by more than the
   rounding the existing GPU test allows.
