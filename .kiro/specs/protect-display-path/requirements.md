# Requirements: protect display_path.rs before it is reduced

Written 2026-11-24 from a plan of lesscode (`step plan`, 20 planted faults in
`src-tauri/src/renderer/display_path.rs`, 1 noticed). Base: `72507e1`. Tests:
`cargo nextest run --locked --manifest-path src-tauri/Cargo.toml --lib --no-fail-fast`.
Not approved yet: the owner reviews once, at the end.

A planted fault is one small mistake made on purpose in a copy of the code. Where every test still
passes with the mistake in place, nothing would tell us if a later change made the same mistake.
The owner chose five functions. Each requirement says what could change unseen there today.

Left alone: `collect` (9 faults, all in code built on Linux only; nothing is known about it on
this system). `precondition` is protected already (1 of 1 noticed).

## Requirement 1: The platform the build is for (`Platform::current`)

Today nothing notices when `current` names the wrong platform (faults `negate@68:11` and
`negate@70:18`).

#### Acceptance Criteria
1. The renderer's tests shall fail when `Platform::current` names a platform other than the one the
   build is for.

## Requirement 2: A fallback is recorded (`DisplayPathState::fall_back`)

Today nothing notices when a fallback leaves the state as it was (fault `drop_statement@448:8`).

#### Acceptance Criteria
1. When a fallback is recorded, the renderer's tests shall fail unless the state then names the
   canvas and gives the reason of the fallback.

## Requirement 3: The backends the layer asks for (`layer_backends`)

Today nothing notices when the layer asks for the other platform's backends (fault
`negate@497:7`).

#### Acceptance Criteria
1. The renderer's tests shall fail when `layer_backends` does not name DX12 on Windows, or Vulkan
   and GL elsewhere.

## Requirement 4: The GPU is tried when nothing rules the layer out (`decide`)

Today nothing notices when `decide` skips the GPU although the layer was asked for (faults
`negate@486:7` and `drop_statement@487:8`), or when it reads another environment variable for
the compositor (fault `arg_literal:string->empty@481:47`).

#### Acceptance Criteria
1. Where the native layer is asked for on Windows, the renderer's tests shall fail when `decide`
   answers without having tried the GPU.
2. The fault at 481:47 concerns a fact that only Linux uses. It is proposed for writing off on
   this system.

## Requirement 5: A failure while the app runs (`runtime_fallback`)

Today nothing notices when `runtime_fallback` does not record the fallback, or does not move the
sessions to the canvas (faults `drop_statement@458:4` and `drop_statement@461:4`).

#### Acceptance Criteria
1. The function takes the handle of the running app. No test of the crate can make one as the
   code stands. Both faults are proposed for writing off as not testable as the code stands; what
   it does to the state is covered by requirement 2.
