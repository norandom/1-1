# Implementation Plan - bounded effort and crate reuse

This is the approved implementation plan; phase approvals are recorded in spec.json.
The original plan has 49 of 61 subtasks completed. Additional harness prerequisites and user-requested professional controls were added during integration. Their individual states are recorded below; original release acceptance remains open.
Current product name is 1-1, pronounced zero. The latest delivery focus is the
per-host profile, compact session menu and viewport-sized initial connection.

## Effort controls
- Execute sequentially within each group; prior groups are prerequisites. No parallel markers: shared manifests, actor code and CI files make parallel work more costly than useful here.
- Normal tasks target 1-3 focused engineering hours; these are planning slices, not delivery promises. At 3 hours without the stated deliverable, record the result and split the remaining work before continuing. Do not hide multi-day work inside a task.
- Feasibility spikes have a hard 3-hour investigation cap each. Missing upstream capability or a second failed integration approach returns to design with evidence; no custom codec, private fork, CI platform, framework rewrite or silent requirement reduction.
- Group checkpoints: tooling/CI parity; old-stack safety net; upstream compile proof; persistence recovery; integrated UI; real-server acceptance. Record actual effort per group and re-estimate remaining work before the next checkpoint.
- TDD applies to behavior changes; retain red/green evidence. Do not build generic mocking, workflow or dependency-management frameworks.
- Keep requirements and design approvals distinct from task generation. Current phase approvals are recorded in spec.json; user-directed amendments are recorded alongside their tasks and evidence.

## Crate and tool choices
- Reuse IronRDP components for codecs/compositor/protocol, Tokio channels/time(test-util) for scheduling and deterministic tests, serde/serde_json for v2 migration, uuid/thiserror/keyring/image and std collections already in the manifest.
- Prefer promoting existing tempfile for same-directory atomic replacement over adding atomic-write-file. Its fit is explicitly tested on all OSes; do not claim durable power-loss transactions from rename alone.
- cargo-nextest is a CI/developer executable, not an application dependency. Keep cargo test --doc; pin a supported release (at least 0.9.150 per current upstream advisory), test resource grouping and Windows process-launch overhead.
- Dagger TypeScript SDK matches the existing TypeScript toolchain. Use Podman or Docker engine through native host CLI. No Rust Dagger SDK, Compose stack, database, graph library, retry framework, criterion, proptest or mockall by default.
- IronRDP 0.5.0 is a proposed EGFX pin from prior design, not a verified compatible set. A compile probe and Cargo.lock are required before adopting it.
- Sources and dependency decisions are recorded in research.md; tasks name outcomes, while design owns code paths.



- [x] 1. Execution tooling and CI contracts
- [x] 1.1 Pin native tool entry points and isolated outputs
  - Inspect wrapper targets before execution; reject WSL/Git Bash delegation. Run from the canonical repository path `D:\Source\drift-rdp` — the `C:\Users\mariu\Source\drift-rdp` junction view must not be used as cwd (detect and reject it so cache keys and BuildArtifacts scoping stay stable). Use configured caches and scoped D:/BuildArtifacts outputs.
  - Done: Native PowerShell preflight reports tool versions/paths and rejects prohibited wrappers.
  - _Boundary: Developer tooling_
  - _Requirements: 7.1,7.5,12.7,13.4_
- [x] 1.2 Configure nextest profiles and retain doctests
  - Use pinned cargo-nextest with no automatic retries, fail on zero tests, timeout/leak reporting and JUnit; serialize only tests sharing OS resources. Keep cargo test --doc.
  - Done: Rust tier command runs nextest and doctests with propagated failures; compare Windows timing rather than assuming speedup.
  - _Boundary: Rust test configuration_
  - _Requirements: 7.1,7.3,13.3,13.4_
- [x] 1.3 Prove native Dagger-to-Podman connectivity
  - Time-box to 3 hours; native Windows CLI connects to the existing Podman engine, which may be WSL-backed. No direct WSL commands, no new VM provisioning or Compose stack.
  - Done: Pinned engine runs a trivial container and exports an artifact under the scoped output directory, or records a blocking compatibility result.
  - _Boundary: Dagger bootstrap_
  - _Requirements: 7.1,12.7,13.4_
- [x] 1.4 Define one command contract for container and native jobs
  - Use a small TypeScript Dagger module with frontend, rust-checks, linux-build and linux-e2e entry points; a native PowerShell entry point invokes the same tier commands for OS-specific checks. Keep toolchain/image versions pinned.
  - Done: Version/config files and command list distinguish Linux container checks from native Windows/macOS validation.
  - _Boundary: CI contract_
  - _Requirements: 7.1,7.2,8.4,12.5,13.3,13.4_

- [x] 2. Dagger Actions foundation on the old protocol stack
- [x] 2.1 Move portable checks into Dagger
  - Reuse npm lockfile and existing Rust commands; provision Linux Tauri libraries once. Cache registry/git and target separately by OS/arch/toolchain/features; never import host target/node_modules.
  - Done: Dagger frontend and Rust checks emit tests/JUnit and fail on an intentionally failing fixture; doctests remain included.
  - _Boundary: Dagger checks_
  - _Requirements: 7.1,7.3,13.3_
- [x] 2.2 Move Linux bundles into Dagger
  - Reuse current Tauri packaging; export AppImage/deb plus logs. Keep cache/output in engine storage and scoped host exports.
  - Done: Linux bundle artifacts exist and missing artifacts fail the job.
  - _Boundary: Linux build_
  - _Requirements: 7.2,7.3_
- [x] 2.3 Prove native Windows cold launch on the old stack
  - 3-hour setup spike: minimum native compile fixes, MSVC/WebView2, pinned Edge driver and isolated data; no protocol upgrade.
  - Done: Built-app Windows cold launch passes or a specific prerequisite blocks upgrade.
  - _Boundary: Windows E2E bootstrap_
  - _Requirements: 8.1,12.1,12.5,12.7,12.8_
- [x] 2.4 Prove native macOS cold launch on the old stack (CI-runner spike)
  - Converted per user decision 2026-11-11 (was a local 3-hour spike; no macOS host exists in this environment): run as an Actions macOS-runner job on the fork. Build on the runner, prove the WebdriverIO Tauri service embedded server on the built app (not safaridriver), isolate data, export evidence artifacts (logs/screenshots/driver output); evidence comes from the live run, recorded in research.md. Keep the 3-hour investigation cap (runner wait excluded).
  - Done: Built-app macOS cold launch passes in CI with exported evidence, or unsupported driver evidence blocks upgrade.
  - _Boundary: macOS E2E bootstrap (CI)_
  - _Requirements: 12.1,12.5,12.7,12.8_
- [x] 2.5 Prove Dagger Linux cold launch on the old stack
  - 3-hour setup spike: WebKitGTK/Xvfb inside the pinned image with isolated app-data; export failure screenshots.
  - Done: Built-app Linux cold launch passes through the Dagger entry point or its prerequisite blocks upgrade.
  - _Boundary: Linux E2E bootstrap_
  - _Requirements: 12.1,12.5,12.7,12.8_
- [x] 2.6 Wire thin Actions jobs and native platform exceptions
  - Dagger action handles portable/Linux jobs; Windows uses PowerShell MSVC/WebView2; macOS uses native bundle/UI jobs. Preserve both macOS architectures. Pin actions/engine; no cloud token required.
  - Done: Workflow retains push/PR triggers, publishes failure reports and artifacts, and requires every gate; Windows contains no bash/wsl shell.
  - _Boundary: Actions integration_
  - _Requirements: 7.1,7.2,7.3,7.4,7.5,8.4,12.5,13.3_
- [x] 2.7 Verify old/new job parity before removing duplicate logic
  - Compare exact commands, features, test inventory and artifacts; remove replaced Linux shell steps only after parity. Keep native checks, not duplicate full Linux runs. Record wall-time and cache-hit observations (non-blocking; cold-build cost accepted for MVP). Evidence wording must never present Linux container results as native Windows/macOS validation.
  - Done: Parity record identifies every old gate and replacement; failed checks still block artifact acceptance.
  - _Boundary: CI migration validation_
  - _Requirements: 7.1,7.2,7.3,12.8_

- [x] 3. Brownfield characterization gate
- [x] 3.1 Classify baseline and pin persistence behavior
  - Classify every Req 1-7 criterion as existing, defect or future delta; 4.4 is not an old-stack gate. Reuse tempfile and serde_json fixtures for CRUD/import/probe/keyring failure seams.
  - Done: Tests preserve fields, secret exclusion, validation, imports and cleanup; missing baseline coverage is explicit.
  - _Boundary: Persistence baseline_
  - _Requirements: 1.1,1.2,1.3,1.4,1.5,6.1,6.2,6.3,12.6_
- [x] 3.2 Pin actor admission and lifecycle behavior
  - Use existing Tokio channels and test-util paused time; extract only a narrow transport seam. Test 9/10/11 admission, retries, cancellation and teardown races.
  - Done: Deterministic lifecycle tests run without sleeps or a replacement actor framework.
  - _Boundary: Session baseline_
  - _Requirements: 2.1,2.2,2.3,2.4,2.5,12.6,13.2_
- [x] 3.3 Pin input and display controls
  - Reuse existing input/display tests; add modifier, pointer scaling, resize debounce, special keys, clipboard and screenshot checks at current seams.
  - Done: Automated tests and required real-server checks cover each control; no assumption that mock frames prove clipboard/NLA.
  - _Boundary: Input baseline_
  - _Requirements: 3.1,3.2,3.3,3.4,3.5,3.6,12.6,12.9_
- [x] 3.4 Pin shell and renderer behavior
  - Reuse Vitest and existing image crate; cover full/dirty frames, GPU-failure canvas path, HUD, themes/settings/toasts/status and shortcuts.
  - Done: Characterization records existing behavior and known defects without freezing incorrect defaults or tab identity.
  - _Boundary: Shell/render baseline_
  - _Requirements: 4.1,4.2,4.3,5.1,5.2,5.3,5.4,12.6_
- [x] 3.5 Record reproducible performance and interoperability baselines
  - Run the design workload matrix and old-stack TLS/NLA smoke; record hardware/build, actual route, server/policy and timings. If a workload cannot run, agree a budget before upgrade.
  - Done: Baseline artifact contains benchmark results and real-server checks; missing mandatory evidence blocks the upgrade.
  - _Boundary: Baseline integration_
  - _Requirements: 2.1,4.1,4.3,7.1,7.2,7.3,12.6,12.9_

- [x] 4. Bounded Rust dependency upgrade
- [x] 4.1 Resolve an upstream-only IronRDP set
  - 3-hour spike: verify the proposed EGFX version against actual registry/source; pin a compatible component set and MSRV in lockfile. Check decode/error callbacks, AVC-disabled flags and compositor output. Prove the decode-error and unsupported-codec hook surface needed by Req 9.6/9.7, and define per-codec decode fixtures for ClearCodec plus every codec CodecPolicy will advertise (Planar, Uncompressed, RFX-Progressive). Do not assume latest documentation matches 0.5.0.
  - Done: Small compile probe demonstrates required upstream APIs and feature graph; decode/error hooks for 9.6/9.7 proven or the advertised caps shrink to proven codecs; per-codec fixtures defined; the resolved set is recorded and group-9 tasks stay blocked until it is. Stop on missing capability, not a custom codec/fork.
  - _Boundary: Protocol dependencies_
  - _Requirements: 9.1,9.2,9.3,9.5,9.6,9.7_
- [x] 4.2 Adapt connector and transport APIs
  - Reuse ironrdp-tokio, TLS/rustls and existing connection flow; no protocol implementation or runtime replacement.
  - Done: Connector compiles and TLS/NLA smoke still succeeds on the pinned stack.
  - _Boundary: Connector adapter_
  - _Requirements: 2.1,8.3_
- [x] 4.3 Adapt session APIs preserving lifecycle
  - Keep actor commands/watch channels and existing reconnect path; adapt DVC accessors and builder only.
  - Done: Nextest characterization and native smoke pass on all supported OSes; actor behavior stays stable.
  - _Boundary: Session adapter_
  - _Requirements: 2.2,2.3,2.4,2.5,3.1,3.2,3.3,3.4,3.5,3.6_
- [x] 4.4 Remove H.264 transport atomically across Rust and UI
  - Explicit integration task: remove encoder, decoder and kind 3 together; reuse current kind-1/2 packet layout.
  - Done: Frontend/backend packet tests pass and cargo tree feature audit contains no openh264 encoder dependency.
  - _Boundary: Frame transport integration_
  - _Requirements: 4.4,9.4,9.5_

- [x] 5. Windows and controlled test sessions
- [x] 5.1 Complete native Windows integration
  - Reuse keyring windows-native, platform cfg and existing wgpu; verify credential round-trip, Windows identity, GPU init/fallback and installer bootstrap. Deliver the explicit NSIS bundle-config change: WebView2 bootstrapper mode pinned in config (downloadBootstrapper default, offline behavior documented); CI fails if the configured mode regresses.
  - Done: Native Windows smoke stores/retrieves credentials, reaches usable canvas and produces a runnable installer with the bootstrapper mode pinned in bundle config.
  - _Boundary: Windows integration_
  - _Requirements: 8.1,8.2,8.3,8.4,8.6_
- [x] 5.2 Normalize shortcut labels and settings defaults
  - Reuse existing platform helper and serde defaults; pin frontend/backend default parity rather than introduce code generation.
  - Done: Windows/Linux Ctrl and macOS Cmd hints work; persisted settings and fresh defaults match.
  - _Boundary: Settings/platform UI_
  - _Requirements: 5.1,5.2,5.4,8.5_
- [x] 5.3 Gate synthetic sessions behind test support
  - Reuse current mock loop, Tokio time and actor state; add loss/cancel controls and build-gated fixture seeding for synthetic saved connections in test builds, with no network. Harness owns isolated app-data/credential namespaces and scoped cleanup; production cannot register or call seeding.
  - Done: Tests cover frames and reconnect/cancel; production rejects persisted or command mock flags.
  - _Boundary: Test session boundary_
  - _Requirements: 12.2,12.7,12.10_
- [x] 5.4 Add build-gated frame observations
  - Count actual emitted packet kinds; register spy/control commands only in test builds.
  - Done: Mock E2E sees bitmap packets; production build lacks test events/commands even with environment flags.
  - _Boundary: Test instrumentation_
  - _Requirements: 12.4,12.10_

- [x] 6. JSON library storage with minimal new code
- [x] 6.1 Model v2 data and tree invariants
  - Reuse serde/serde_json, uuid and std HashMap/HashSet. Keep flat entities with parent IDs; no graph crate, database or ORM.
  - Done: Tests validate unique IDs, ownership, ancestor cycles, deep nesting and descendant queries.
  - _Boundary: Library model_
  - _Requirements: 10.1,10.5,10.8_
- [x] 6.2 Prove atomic replacement with the existing tempfile crate
  - 3-hour spike: promote tempfile to runtime use only if NamedTempFile new_in/persist fits replacement and failure behavior on all OSes. Use a narrow writer wrapper; atomic visibility is not a claim of power-loss durability.
  - Done: Tests preserve prior file on failed replacement, use same-directory staging and retain recoverable error state; if unsupported, return to design before a second crate.
  - _Boundary: Atomic writer_
  - _Requirements: 6.1,10.10,10.12_
- [x] 6.3 Implement migration and write-blocked recovery
  - Reuse versioned serde fixtures; preserve every field and ID; create backup without clobber, fail closed on existing invalid backup or unsupported schema.
  - Done: Fault tests cover backup/commit failure, malformed data, interrupted staging and restart; recovery cannot save an empty replacement.
  - _Boundary: Migration_
  - _Requirements: 6.1,6.2,10.10,10.12_
- [x] 6.4 Implement library and folder CRUD/read models
  - Keep one store owner and serialized transactions; register thin commands and typed wrappers.
  - Done: Create/rename/read operations round-trip with nested descendants; errors preserve prior snapshot.
  - _Boundary: Library commands integration_
  - _Requirements: 10.1,10.2,10.3,10.5,10.7_
- [x] 6.5 Implement atomic cross-library moves
  - Validate proposed snapshot then persist/publish; preserve IDs, credentials, settings and active session references.
  - Done: Cycle/missing-target rejection, subtree moves and failed-write rollback pass tests.
  - _Boundary: Tree transactions_
  - _Requirements: 10.8,10.15_
- [x] 6.6 Integrate connection CRUD and duplication
  - Adapt existing validation/import/probes; allocate fresh duplicate ID/name without keyring copying. Keep mock data inert in production.
  - Done: Existing CRUD/import tests pass in v2; duplicate has no copied password and correct folder.
  - _Boundary: Connection store integration_
  - _Requirements: 1.1,1.2,1.3,1.4,1.5,6.2,10.7,10.19,12.10_

- [x] 7. Session identity and destructive operations
- [x] 7.1 Make connect admission idempotent by connection
  - Reuse SessionManager map and Tokio command serialization; return the existing live session ID or restart a terminal entry with stable ID; UI focus is owned by the later tab integration task.
  - Done: Concurrent Connect requests yield one actor and returned session ID; explicit close then Connect gets a new identity.
  - _Boundary: Session identity_
  - _Requirements: 2.2,10.4,10.6,11.4,11.8,11.9_
- [x] 7.2 Delete folders without deleting connections
  - Remove entire folder subtree and move descendants to library root in a single store transaction.
  - Done: Exact removed/moved counts match preview; credentials and active sessions survive failed/successful operations.
  - _Boundary: Folder deletion_
  - _Requirements: 10.16,10.18_
- [x] 7.3 Coordinate connection/library deletion with actors
  - Explicit integration task: serialize affected connect/delete operations, cancel retries, await stop up to five seconds, persist deletion then clean keyring best-effort.
  - Done: Timeout/persist failure retains saved data; race/restart tests confirm stopped-session and cleanup semantics.
  - _Boundary: Deletion integration_
  - _Requirements: 1.5,6.2,10.2,10.17,10.18_
- [x] 7.4 Reactivate deactivated sessions in place
  - User decision 2026-11-14 (group-4 hold item 3): adopt the proven upstream mechanism (activation_factory + ActiveStage::reactivate()) so server DeactivateAll reactivates the existing session in place, replacing the pinned DeactivateAll -> ConnectionLost -> backoff-reconnect behavior; flip the suite pins that freeze the old observable behavior. Surface any Req 2 wording change at the group-7 boundary HOLD before landing.
  - Done: DeactivateAll during an active session reactivates in place with stable session identity (no ConnectionLost/backoff cycle); unexpected-drop reconnect and cancel behavior unchanged.
  - _Boundary: Session reactivation_
  - _Requirements: 2.3,2.4,11.4_

- [x] 8. Tree, dashboard and tab UX
- [x] 8.1 Pin annotated interaction states as component fixtures
  - Use existing Radix/shadcn and Vitest patterns; cover tree/rail/filter, focus/menus, dashboards with tabs. Avoid introducing another component system.
  - Done: UX acceptance states define expected keyboard, status and focus outcomes before component changes.
  - _Boundary: UX fixtures_
  - _Requirements: 10.13,10.14,11.6,11.7_
- [x] 8.2 Build typed tree state and node-aware editing
  - Reuse Zustand, current form and IPC wrappers; persist selection/expansion; filter includes ancestors and restores prior state.
  - Done: Store tests cover filter/selection; create/edit uses selected destination; QuickConnect aggregates libraries.
  - _Boundary: Tree state integration_
  - _Requirements: 10.1,10.3,10.9,10.11,1.1,1.2_
- [x] 8.3 Render accessible tree and context actions
  - Reuse Radix primitives and existing status subscriptions; implement roving focus, named status, explicit rail toggle and confirmations.
  - Done: Keyboard/component checks cover arrows/Home/End/Enter/Shift+F10, all menus, duplication and confirmation counts.
  - _Boundary: Tree UI_
  - _Requirements: 10.3,10.4,10.7,10.11,10.13,10.14,10.18,10.19_
- [x] 8.4 Add movement through one shared action
  - Gate dnd-kit on a hard-capped (3-hour) React 19 peer-compatibility spike with a recorded go/no-go before this task; on no-go, use native HTML5 drag. The keyboard Move dialog shares the same action either way, and the native fallback must retain the keyboard equivalent.
  - Done: Drag and keyboard subtree moves persist; rejected writes restore visible location and focus.
  - _Boundary: Move UI_
  - _Requirements: 10.8,10.14,10.15_
- [x] 8.5 Connect dashboards and start view
  - Reuse card/form/quick-connect patterns; select nodes without closing sessions and show descendants.
  - Done: Dashboard double-click and tree Connect share one action; empty/no-selection states and live status are correct.
  - _Boundary: Dashboard integration_
  - _Depends: 7.1_
  - _Requirements: 10.5,10.6,11.5,11.6_
- [x] 8.6 Hoist persistent session tabs and fix routes
  - Reuse session store; use session ID routes and named keyboard close controls; wire active/hover/focus states and consume backend existing-session IDs to focus the exact tab.
  - Done: Two-session tests switch/close/reconnect exactly the intended session and restore adjacent/dashboard focus.
  - _Boundary: Tab integration_
  - _Depends: 7.1_
  - _Requirements: 11.1,11.2,11.3,11.4,11.5,11.6,11.7,11.8,11.9_
- [x] 8.7 Expose store recovery and disable mutation controls
  - Bind backend recovery state to a persistent instructions view; disable CRUD/import/move/autosave without losing stored originals. Restore controls only after valid restore/reload.
  - Done: Corrupt-file startup and failed migration E2E show instructions and reject every mutation path; valid recovery reload restores the UI.
  - _Boundary: Recovery UI integration_
  - _Depends: 6.3_
  - _Requirements: 10.12,10.15_

- [x] 9. Upstream EGFX and bounded rendering
- [x] 9.1 Implement capability policy from proven upstream APIs
  - Use only supported non-AVC flags/versions from the pinned crate; distinguish channel selection and observed update codec.
  - Done: Tests cover supported EGFX codecs, no-EGFX bitmap fallback and absence of AVC advertisement.
  - _Boundary: Codec policy_
  - _Requirements: 9.1,9.3,9.5_
- [x] 9.2 Connect upstream compositor to existing frame paths
  - Reuse GraphicsPipelineClient surface lifecycle and frame acknowledgement; adapt decoded output to SharedFrame and current packets, no custom codec.
  - Done: Fixtures cover ClearCodec and allowed alternate codecs plus surface reset/resize; GPU/canvas receive matching pixels.
  - _Boundary: EGFX adapter integration_
  - _Requirements: 4.1,4.2,9.2,9.4_
- [x] 9.3 Make unsupported/decode errors terminal
  - Use observable upstream error/unhandled hooks from dependency spike; avoid protocol parser duplication.
  - Done: Unsupported AVC terminates without retry; three failed frames terminate and successful decode resets counter.
  - _Boundary: Codec error policy_
  - _Requirements: 9.6,9.7_
- [x] 9.4 Batch dirty regions without a new buffering crate
  - Use existing framebuffer and rectangle math; union dirty bounds, choose rectangle/full packet at reset and initial delivery.
  - Done: Disjoint/overlapping updates and resize fixtures produce correct complete pixels.
  - _Boundary: Frame batching_
  - _Requirements: 4.1,4.4,9.8_
- [x] 9.5 Bound UI delivery with existing Tokio primitives
  - One in-flight packet plus pending damage accumulator; generation-scoped consumption acknowledgement; five-second stall and full refresh on recovery.
  - Done: Delayed/dropped/stale acknowledgement tests preserve latest full image and bound pending delivery.
  - _Boundary: Frame acknowledgement integration_
  - _Requirements: 9.8,4.1,4.3_

- [x] 10. Font smoothing and display fidelity
- [x] 10.1 Persist and import the smoothing preference
  - Reuse serde default and mirrored types, form and existing RDP parser; preserve explicit false and legacy default true.
  - Done: Import/save/reload/migration tests pass for missing/true/false preferences.
  - _Boundary: Connection preferences_
  - _Requirements: 14.1,14.3_
- [x] 10.2 Apply font-smoothing flags and policy help
  - Use upstream performance flags; do not conflate codec with server smoothing acceptance.
  - Done: Both flag states verified; help explains policy override and next-connect application.
  - _Boundary: Font smoothing integration_
  - _Requirements: 14.2,14.4_
- [x] 10.3 Implement native-pixel and fit coordinates
  - Reuse SessionCanvas and input mapper; handle devicePixelRatio, viewport scrolling and DPI changes once.
  - Done: Text fixtures match decoded pixels in native mode; pointer/scroll mapping passes at four DPIs in both modes.
  - _Boundary: Display integration_
  - _Requirements: 3.2,4.1,14.5_

- [ ] 11. Regression journeys and release evidence

User-directed priority: finish the professional controls below before resuming the release harness.
- [x] 11.0d Persist professional host display/experience options and apply supported RDP flags. Defaulted typed profile, truthful network hints, standard .rdp import, saved explicit values/duplication/reopen, flag polarity and unchanged security/codec policy verified. _Boundary: Connection persistence and connector configuration. Requirements: 14.7,14.8._
- [x] 11.0e Move display controls into compact accessible session menu and expose per-host options in connection settings. No extra display row, saved local fit/native and dynamic/fixed behavior, presets, next-connect help, local menu input isolation and visible save failures. _Boundary: Host settings and session UI. Depends: 11.0d. Requirements: 14.5,14.7,14.8._
- [x] 11.0f Size initial dynamic connections from the actual viewport through existing IPC. Correct DPR/bounds, fixed-mode preservation, no transient size persistence, first-connect/restart/navigation regression coverage. _Boundary: Session viewport and connection admission. Depends: 11.0e. Requirements: 3.3,14.9._

- [x] 11.0g Add a persisted status-bar auto-hide setting and readable host labels. Reclaim session height when hidden; reserve layout space on reveal and resize dynamic sessions through the existing debounce to prevent overlap. Preserve fixed resolution and stable bottom-edge/keyboard access, and retain normal status display outside sessions. Verify legacy/default/false settings persistence and all lifecycle labels without internal IDs. _Boundary: Application settings and status bar. Requirements: 14.11,14.12._
- [x] 11.0h Add supported per-host colour-depth and bulk-compression controls, with accurate scope/reconnect help, persistence/import and connector mapping. _Boundary: Host profile, connection form/session menu, connector configuration and relevant tests. Requirements: 14.13,14.14. User-directed amendment approved 2026-11-17._
- [x] 11.0i Delay auto-hidden status-bar pointer reveal by one second; keep keyboard reveal immediate and cancel pending timers safely. _Boundary: Status bar and focused tests. Requirements: 14.15. User request 2026-11-17._
- [x] 11.0j Add reliable clipboard direction controls, active-session isolation, operational status and status-bar Resync clipboard using existing text support; handle reconnect/failures truthfully and preserve remote-input boundaries. _Boundary: Clipboard backend/session commands, shared host settings, frontend clipboard controller and status bar. Requirements: 14.16,14.17,14.19. User-approved direction 2026-11-17._
- [x] 11.0k Add bounded bidirectional image/screenshot clipboard transfer using supported formats and local clipboard facilities; verify repeated transfers and failure recovery. _Boundary: Clipboard conversion/backend, typed IPC and frontend/native clipboard integration. Depends: 11.0j. Requirements: 14.18,14.19. User-approved direction 2026-11-17._

Release-harness prerequisites (remaining):
- [ ] 11.0a Establish the shared harness and Windows adapter
  - Prerequisite repair identified on 2026-11-17: the shared harness assumed by 11.1 does not exist. Reuse the approved WebdriverIO/native Windows driver approach, pinned dependencies and lockfile; introduce the shared command/spec directory, isolated app/settings/credential resources, cold-launch UI assertion, bounded teardown and failure artifacts.
  - Done: The built Windows app passes the shared cold-launch spec; an intentional failure propagates with evidence; only owned resources are cleaned up. Keep the existing three-hour investigation/split threshold.
  - _Boundary: Windows E2E bootstrap and shared harness_
  - _Requirements: 12.1,12.5,12.7,12.8_
- [ ] 11.0b Complete the approved macOS harness adapter
  - Implement the already-approved debug-gated tauri-plugin-wdio-webdriver dependency/capability, run the shared cold-launch spec and require its native CI result. Preserve production exclusion; do not select another driver.
  - Done: Native macOS evidence proves UI control, intentional-failure propagation and teardown; release dependency/configuration checks exclude test control. Keep the existing three-hour investigation/split threshold.
  - _Depends: 11.0a_
  - _Boundary: macOS E2E bootstrap_
  - _Requirements: 12.1,12.5,12.7,12.8,12.10_
- [ ] 11.0c Complete Linux harness and matrix integration
  - Extend the existing Dagger entry with the approved WebKitGTK/Xvfb route and shared cold-launch spec; preserve failure evidence and require all three platform results.
  - Done: Dagger Linux UI execution and intentional-failure propagation pass; the same spec runs in all three jobs. Keep native Windows/macOS evidence distinct. Keep the existing three-hour investigation/split threshold.
  - _Depends: 11.0a,11.0b_
  - _Boundary: Linux E2E bootstrap and matrix integration_
  - _Requirements: 12.1,12.5,12.7,12.8,13.3_
- [ ] 11.1 Verify CRUD, migration and recovery journeys
  - Implement/rerun create/edit/import/duplicate/delete and v1/recovery journeys using the existing harness; run the same specs in the three OS jobs.
  - Done: Fixtures preserve fields and secrets; rollback/recovery mutations fail safely and restart state is correct.
  - _Boundary: Storage E2E_
  - _Depends: 11.0a,11.0b,11.0c; reviewed 8.3 library-creation remediation_
  - _Requirements: 1.1,1.2,1.3,1.4,1.5,10.1,10.2,10.7,10.10,10.12,10.16,10.17,10.18,10.19,12.3,12.5,12.7_
- [ ] 11.2 Verify tree and keyboard journeys
  - Run filter/ancestor, rail persistence, keyboard menus/Move, subtree move and dashboard scenarios against the shared harness.
  - Done: All three OS jobs cover successful and rejected moves with correct focus, status and visible hierarchy.
  - _Boundary: Tree E2E_
  - _Requirements: 10.3,10.4,10.5,10.6,10.8,10.9,10.11,10.13,10.14,10.15,12.3,12.5_
- [ ] 11.3 Verify session, settings and production-build journeys
  - Cover concurrent Connect, tab switch/close/reconnect, dashboard with live sessions, themes/settings restart and build-feature exclusion.
  - Done: Three OS jobs exercise only intended session changes; release builds reject mocks and expose no test controls/events.
  - _Boundary: Session E2E_
  - _Requirements: 5.1,5.2,5.3,11.1,11.2,11.3,11.4,11.5,11.6,11.7,11.8,11.9,12.3,12.4,12.5,12.10_
- [ ] 11.4 Verify Windows workstation interoperability
  - Run a checklist against one provisioned Windows workstation target: NLA where applicable, retry/cancel, input, clipboard, resize, actual codec observations. Reuse fixtures and logs; record server-specific applicability. Additional server versions are separately scheduled executions, not hidden work.
  - Done: One target report identifies client/server builds, required results and actual route; missing targets or applicable checks remain blockers.
  - _Boundary: Real-server validation_
  - _Requirements: 2.1,2.3,2.4,3.1,3.2,3.3,3.4,3.5,3.6,9.1,9.2,9.3,12.9_
- [ ] 11.5 Verify Windows Server interoperability
  - Run a checklist against one provisioned Windows Server target: NLA where applicable, retry/cancel, input, clipboard, resize, actual codec observations. Reuse fixtures and logs; record server-specific applicability. Additional server versions are separately scheduled executions, not hidden work.
  - Done: One target report identifies client/server builds, required results and actual route; missing targets or applicable checks remain blockers.
  - _Boundary: Real-server validation_
  - _Requirements: 2.1,2.3,2.4,3.1,3.2,3.3,3.4,3.5,3.6,9.1,9.2,9.3,12.9_
- [ ] 11.6 Verify grd interoperability
  - Run a checklist against one provisioned grd target: NLA where applicable, retry/cancel, input, clipboard, resize, actual codec observations. Reuse fixtures and logs; record server-specific applicability. Additional server versions are separately scheduled executions, not hidden work.
  - Done: One target report identifies client/server builds, required results and actual route; missing targets or applicable checks remain blockers.
  - _Boundary: Real-server validation_
  - _Requirements: 2.1,2.3,2.4,3.1,3.2,3.3,3.4,3.5,3.6,9.1,9.2,9.3,12.9_
- [ ] 11.7 Verify text fidelity at 100% client DPI
  - Batch fixed text cases with smoothing on/off, native/fit modes and EGFX/bitmap fallback at 100% DPI. Reuse provisioned targets and capture procedure; extra client OS executions are explicit instances with recorded effort.
  - Done: all eight combinations have a report separating decoded/canvas pixel equality, OS screenshots and physical observations; policy overrides and unavailable routes remain explicit.
  - _Boundary: Text fidelity validation_
  - _Requirements: 12.9,14.4,14.5,14.6_
- [ ] 11.8 Verify text fidelity at 125% client DPI
  - Batch fixed text cases with smoothing on/off, native/fit modes and EGFX/bitmap fallback at 125% DPI. Reuse provisioned targets and capture procedure; extra client OS executions are explicit instances with recorded effort.
  - Done: all eight combinations have a report separating decoded/canvas pixel equality, OS screenshots and physical observations; policy overrides and unavailable routes remain explicit.
  - _Boundary: Text fidelity validation_
  - _Requirements: 12.9,14.4,14.5,14.6_
- [ ] 11.9 Verify text fidelity at 150% client DPI
  - Batch fixed text cases with smoothing on/off, native/fit modes and EGFX/bitmap fallback at 150% DPI. Reuse provisioned targets and capture procedure; extra client OS executions are explicit instances with recorded effort.
  - Done: all eight combinations have a report separating decoded/canvas pixel equality, OS screenshots and physical observations; policy overrides and unavailable routes remain explicit.
  - _Boundary: Text fidelity validation_
  - _Requirements: 12.9,14.4,14.5,14.6_
- [ ] 11.10 Verify text fidelity at 200% client DPI
  - Batch fixed text cases with smoothing on/off, native/fit modes and EGFX/bitmap fallback at 200% DPI. Reuse provisioned targets and capture procedure; extra client OS executions are explicit instances with recorded effort.
  - Done: all eight combinations have a report separating decoded/canvas pixel equality, OS screenshots and physical observations; policy overrides and unavailable routes remain explicit.
  - _Boundary: Text fidelity validation_
  - _Requirements: 12.9,14.4,14.5,14.6_
- [ ] 11.11 Verify performance and memory budgets
  - Use baseline workload and same machine; reuse image crate for pixel diffs and std Instant/logs, no benchmark framework unless justified by failed measurement.
  - Done: Recorded p95/memory/queue budgets meet design; delayed consumers recover full-image equality.
  - _Boundary: Performance validation_
  - _Requirements: 4.1,4.2,4.3,9.8_
- [ ] 11.12 Run final native/container gates and evidence audit
  - Collect already-defined nextest plus doctests, frontend suites, fmt/clippy/check, all installers and native first-usable-state smoke; keep Windows/macOS exceptions explicit. Evidence wording must never present Linux container results as native Windows/macOS validation.
  - Done: Every AC has fresh evidence; no blocked tasks, skipped required jobs or test-first audit gaps; tier commands are documented.
  - _Boundary: Final validation_
  - _Requirements: 7.1,7.2,7.3,8.1,8.2,8.3,8.4,8.5,8.6,12.1,12.6,12.8,12.9,13.1,13.2,13.3,13.4_

## Implementation Notes
- User-directed additions (2026-11-17, requirements 14.7-14.10) follow the reviewed smart-resize repair sequentially: branding; 11.0d persisted host profile and protocol flags; 11.0e compact menu and disconnected host settings; 11.0f viewport-sized initial connect. These are explicit additions, not evidence that original acceptance tasks are complete.
- 2026-11-17: Smart-resize remediation independently APPROVED: DisplayControl joins the existing DRDYNVC, latest pending requests wait for readiness, and reported dimensions follow server output. 298 default/316 test-support Rust tests pass (four existing skips), doctests/fmt/Clippy pass (two existing warnings). Existing smoke now records bounded shrink/restore confirmation from server activation or EGFX reset plus decoded desktop output; cursor-only updates are excluded by a documented smoke-only cursor policy. Native real Windows connection and resize from 1920x1080 to 3180x1271 with desktop redraw VERIFIED. User Escape interrupted restore; FreeRDP/IronRDP resize and full new harness run remain pending. Test app subsequently confirmed closed; three isolated credentials removed and Computer Use reset. New user follow-up: compact menu instead of extra display bar, per-host visual/network options, viewport-sized initial connection, and rename to 1-1 (pronounced zero).
- 2026-11-17: B3-7 prerequisite remediation independently APPROVED in Rust and frontend. Failed real connects no longer become synthetic Connected; retry errors persist and pending reconnect dial is cancellable. Native testing exposed and fixed the Rust error-object/string mismatch and presence-based Connected footer. VERIFIED native invalid-host result: Connection Error plus DNS cause, Error in tree/tab/footer, zero active sessions; explicit mocks still render. 291 default Rust/309 test-support (four existing skips), 487 frontend, lint/typecheck/build/fmt/doctests/Clippy pass (two existing warnings). Three staged real hosts passed connector smoke and first-frame/shutdown gates; display-control DVC was unavailable in that harness, so no smart-resize or text-quality acceptance is claimed. Next per user: real app connection and smart-resize on all three hosts. No font defect is currently reported. Computer Use closed/reset; original 49/61 count unchanged. Evidence: `D:/BuildArtifacts/drift-rdp/group11`.
- 2026-11-17: 10.3 display-control remediation independently APPROVED: global shortcut matching excludes Alt chords, production canvas keyboard lifecycle has regression coverage, theme foregrounds are paired, and overlays are clipped below the control row outside native scrolling. VERIFIED: 475 frontend tests, lint/typecheck/build; native label visibility, unobstructed selector and selecting Native pixels with rendered mock frame. Native shortcut focus remains inconclusive through Computer Use; a separate Ctrl+Shift+P probe opened WebView2 Print (cancelled). Carry this native keyboard gap into 11.2; do not claim full native shortcut acceptance. Test app closed and Computer Use reset. Evidence: `D:/BuildArtifacts/drift-rdp/group11/display-*`.
- 2026-11-17: 8.3 New Library remediation independently APPROVED after correcting collapsed-rail refresh-error visibility. Creation uses existing IPC, recovery/pending guards and retained saved receipts; persistent global warnings prevent duplicate retries after refresh failure. Selected tree rows now use paired contrast tokens. VERIFIED: 471 frontend tests, lint/typecheck/build; native isolated creation, Enter submit, dashboard selection, rail open/cancel, restart persistence and readable selected name. Native group-10 follow-up also verified smoothing off save/reopen and stored false. Computer Use was closed/reset after each bounded check. Display-mode smoke exposed faint light-theme labels and a shortcut collision with the performance HUD; remediate 10.3 before harness work. Artifacts: `D:/BuildArtifacts/drift-rdp/group11`.
- 2026-11-17: Group 11 resumed. 11.1 preflight and independent kiro-debug found omitted prerequisites, not a new driver/product decision: no shared E2E suite exists and New Library has no UI caller. Debug verdict RETRY_TASK routes library creation to the already-authorized 8.3 remediation, then explicit 11.0a-c harness slices before unchanged Storage E2E scope. These three prerequisite slices are additional to the original 61 subtasks; the original completion count remains 49/61. No new acceptance result is claimed from preflight.
- 2026-11-17: 10.3 independently APPROVED after fixing passive wheel handling. Native dimensions use DPR once; fit preserves aspect ratio; shared canvas bounds account for scrolling. Persistent display control preserves remote Tab and releases forwarded keys before local focus. Non-passive wheel subscription cancels local scrolling and forwards once, with cleanup coverage. VERIFIED implementation: 463 frontend tests, lint/typecheck/build; group backend regression 307 test-support tests passed (four existing skips), native test binary built. Four-DPI full/dirty RGBA, input, and wheel lifecycle evidence: `D:/BuildArtifacts/drift-rdp/group10/pixel-*.log`. Native UI launch was rejected by automatic approval policy ("blocked by policy", no further reason); no app or Computer Use session started. Native UI smoke, actual canvas readback and physical/server text-quality acceptance remain pending in 11.x. Group 10 checkpoint: 49/61 original subtasks complete; no release GO.
- 2026-11-17: 10.2 independently APPROVED. Connector sets/clears upstream ENABLE_FONT_SMOOTHING from saved preference; XOR test proves only that performance bit changes. Accessible form help describes next connection and server override. VERIFIED: 445 frontend/289 Rust tests (four existing skips), lint/typecheck/build/fmt/doctests and Clippy with two existing warnings. No ClearType/server-acceptance claim. Original RED transcript copied to `D:/BuildArtifacts/drift-rdp/group10/flags-red-transcript.txt`.
- 2026-11-17: 10.1 independently APPROVED. Default-on serde/read models and form switch preserve explicit false through `.rdp` import, create/update, restart, v1 migration, v2 load and duplicate. VERIFIED: 445 frontend/288 default Rust tests (four existing skips), lint/typecheck/build/fmt/doctests; test-support Clippy passes with two existing warnings. Prior ConnectionForm tree-refresh fix is preserved in this commit. Protocol application/help remain 10.2; native integrated UI verification follows 10.3. Evidence: `D:/BuildArtifacts/drift-rdp/group10/preference-*.log`.
- 2026-11-17: 9.5 independently APPROVED after reactivation remediation. One in-flight receipt plus accumulated damage; ordered same-channel metadata keeps binary bytes unchanged; active canvas acknowledges after painting, background runtime after compositing. Five-second stall/full-refresh recovery and smaller/same-size reactivation are covered by real actor pixel tests. VERIFIED: 443 frontend tests, 286 default Rust and 304 test-support Rust tests (four existing skips each), lint/typecheck/build/fmt/doctests, Clippy with two existing warnings. Final native test-support smoke verified dashboard return, two-session rendering and adjacent close; two explicit connects/two actors. Artifacts: `D:/BuildArtifacts/drift-rdp/task95`. Group 9 checkpoint; next 10.1. Native synthetic evidence is not real-server text-quality acceptance.
- 2026-11-17: 8.6 remediation independently APPROVED after one rejected round. Session-owned channels/latest composed image survive view navigation; connection-keyed views reset input state; synchronous route/removal fixes adjacent-tab close. VERIFIED: 438 frontend tests, lint/typecheck/build; native dashboard return and close B to existing A with pixels retained. No checkbox count change (45/61). Native failure acceptance remains unproven because existing B3-7 backend fallback turns failed real connects into mock Connected sessions; resolve before 11.x server acceptance. Roadmap: `.kiro/steering/roadmap.md`.
- Controller sweep owed (before 11.12 audit): baseline-coverage.md rows need updating for 6.6 — B3-1/B3-2/B3-3 FIXED (regression tests in library.rs connection_crud_tests), B3-4 OBSOLETED (fail-closed v2 store), row 1.3/B3-5 pin intact (restored verbatim); B3-2 keyring-half remains convention+E2E covered (see test doc at library.rs ~L4683).
- 6.5 suggestion carried for 8.4/11.2: add a direct folder→library-root move test (parent→None + subtree library_id rewrite) — branch is shared code already proven via connection→library, but deserves its own case.
- 6.4 design alignments landed 2026-11-14: design.md now records the typed `descendants` error, the free-fn `migrate_v1` shape, the two recovery read commands, and the transitional `libraries.json` layout. **6.6 obligation**: unify `libraries.json` + v1 `connections.json` — merge v1 connections into the (possibly already user-populated) v2 default library without data loss; promote `LibraryStore::document()/save()` from `#[cfg(test)]` if a whole-document production caller appears.
- 6.1 model layer notes for the store tasks: `LibraryError` is hand-rolled (Display + std::error::Error) though thiserror sits unused in Cargo.toml — either switch to the derive or record the hand-roll decision when 6.3+ adds error types; `LibraryDocument::descendants` returns a typed `UnknownNode` error where design L346 shows a bare Vec on `LibraryStore` — 6.4 must reconcile (map at the store layer or amend design); `#[allow(dead_code)]` on `pub mod library` must be dropped when the first production caller lands (6.3-6.6).
- Sub-agent verification claims must paste VERBATIM tool output (5.4 round 1: a name-filtered clippy summary hid `(lib) generated 2 warnings` dead-code lines behind the two pre-existing warning texts, producing a false "exactly 2 warnings both configs" claim). Reviewers: always re-run static checks with full output.
- Pre-existing baseline: `npm run lint` fails at HEAD (7 errors / 2 warnings in `src/**` — ConnectionCard.tsx, SessionCanvas.tsx, ui/{badge,button,tabs,toggle}.tsx, useClipboard.ts); must be fixed before the lint tier can gate CI (found during 1.1 review; not caused by 1.1).
- design.md File Structure Plan listed `scripts/Invoke-CI.ps1` but not `scripts/Invoke-CI.Tests.ps1`; RESOLVED 2026-11-13 (group-5 docs alignment — test file added to the plan).
- nextest os-keyring serialization: RESOLVED by task 5.1 — config relocated to `src-tauri/.config/nextest.toml` (workspace root auto-discovers it; bare runs, `--profile dev/ci`, and the tier all serialize real-vault tests via explicit `[[profile.default]]`/`dev`/`ci` overrides; keep the three blocks in sync; `profile.default.extends` is rejected by nextest 0.9.150). Still true: host sets CARGO_TARGET_DIR=D:\BuildArtifacts\cargo-target; nextest junit resolves under the workspace target dir.
- Lint baseline detail (for 2.1): clean checkout has 9 problems in `src/**`; any dagger CLI run generates `.dagger/sdk/**` (gitignored) which `eslint .` then picks up, adding 34 problems. 2.1 must repair the 9 src problems AND add `.dagger` (or `.dagger/sdk`) to eslint.config.js global ignores so clean vs post-dagger runs agree.
- `.dagger/src/index.ts` linuxE2E: once `e2e/` exists, the e2e-present path must throw until 2.5+ wires the actual run (today it correctly fails loudly while `e2e/` is absent).
- CT04d cross-checks image tags between pins.ts and ci-versions.json but not the `@sha256:` digests — digest drift would go unnoticed; tighten when touched.
- Invoke-DaggerTier writes one `dagger-tier-report.json` per run regardless of entry (last-writer-wins); use per-entry report names when 2.1 turns these into CI artifacts. Per-entry evidence survives in `entry-<name>-output.txt`.
- Cosmetic: Invoke-CI.Tests.ps1 header comment says "CT01-CT11"; actual sections are CT01-CT10.
- 2.1: lint baseline repaired (9 src/** problems; .dagger/sdk globally ignored) — `npm run lint` now gates CI. Dagger CLI sync itself already excludes node_modules/untracked (task-1.4 README claim corrected); explicit staging + module guard remain the never-import contract. cargo-nextest tarball ships without a leading directory — installer now self-verifies with `cargo-nextest --version`.
- 2.2: linux-build gates missing bundles both module-side (throw) and tier-side (exit 1, ok=false report). Review hardening ideas (non-blocking): clear `src-tauri/target/<triple>/release/bundle` before build (stale-bundle blind spot only on warm dev engines; CI runners ephemeral); persist red-phase transcripts next to `linux-build-run-*.txt` before the 11.12 evidence audit; container apt extras `file`+`libfuse2` are insurance GH runners already ship (lists differ, disclosed).
- 2.3 spike PROVEN (zero code changes; research.md section + D:\BuildArtifacts\drift-rdp\windows-cold-launch\). Key handoffs for 2.5: WEBVIEW2_USER_DATA_FOLDER isolates WebView2 state but Tauri app-data uses Known Folder APIs (env overrides ignored — need identifier-overlay or own-resource-cleanup); msedgedriver must be resolved at run time from the installed runtime's registry pv (static pins rot — Edge Stable moved mid-spike); real window discovery must enumerate by pid+size (tao 16×16 helper-window trap); graceful shutdown = WM_CLOSE to the real window. wgpu picked Vulkan (not DX12) on the host.
- 2.5 spike PROVEN (research.md section + D:\BuildArtifacts\drift-rdp\linux-cold-launch\). Pattern: e2e-style entries return an evidence Directory instead of throwing on failure (a thrown dagger call cannot chain export — evidence would be lost); the TIER gates on exported cold-launch-status.json (ok=false / missing / invalid / ok-without-screenshot → exit 1). AppImage runs via --appimage-extract-and-run (no FUSE); wgpu falls back to Mesa llvmpipe. Graceful WM_DELETE_WINDOW under bare Xvfb exits 1 with a teardown-only GDK X-error (first-usable-state still proven) — consider a lightweight WM when WebdriverIO lands (tasks 12.x). Retire or CI-gate the DRIFT_E2E_BREAK_LAUNCH sabotage seam when the real suite lands.
- 2.6: workflow reworked into 6 thin jobs (4 dagger/* via dagger-for-github@27b130bf CLI 0.21.11 + runner-side twin gates, windows-native pwsh-only, macos-bundle dual-arch). CT14 27 assertions; suite 222/0. Linux runners use their own Docker engine (Invoke-CI -Tier dagger stays dev-host-only; twin gates mirror Invoke-CI.ps1 L1062-1133). OBLIGATIONS: (a) 2.7 must actually perform the parity comparison against the pre-rework ci.yml at HEAD 344fe41 (old Linux shell jobs were removed in 2.6; design says verify parity before removal — recoverable via git, comparison remains executable); (b) optional hardening: add screenshot-exported guard to the linux-e2e runner twin (dev-host tier checks it at Invoke-CI.ps1 L1122); (c) first live Actions run on push confirms windows-2025 D:\BuildArtifacts writability + NSIS/dmg upload globs.
- 2.7: parity record in research.md L257+ (12 rows vs git show 344fe41 workflow: 3 PARITY / 9 PARITY-WITH-DELTA / 0 GAP) — 2.6 obligations (a) parity comparison and (b) screenshot guard (CT14ab, RED 222/1 → GREEN 223/0) are DONE; obligation (c) first live Actions run still open. Nits for future doc touch: row-12 wording could clarify native jobs dropped host caching (net effect cache-cold, MVP-accepted); cold full-build wall time not durably recorded (2.2 cold report overwritten by later same-entry runs — use per-entry unique names or preserve cold evidence next time).

- 2026-11-17: 11.0d independently APPROVED. Defaulted host profiles preserve legacy updates and explicit false values through CRUD, import, duplication and reopen; supported visual flags and six network hints map to the connector. VERIFIED: 304 default / 322 test-support Rust tests (four existing skips each), 490 frontend tests, docs/fmt/lint/typecheck/Clippy pass (two existing warnings). Evidence: D:/BuildArtifacts/drift-rdp/group11/review-profile-*.log. Menu and initial viewport sizing remain 11.0e/f; no new real-server acceptance claimed.

- 2026-11-17: 11.0e independently APPROVED after portalling nested menu panels to avoid clipping. Existing document chrome now hosts display/visual/network options; no extra display row. Shared form options, serialized latest-state updates, visible save errors, local input isolation and fixed-resize cancellation verified. Full frontend 503 tests/lint/tsc/Vite pass; portal remediation 14 focused tests/tsc/lint pass. Native visual/keyboard acceptance pending. Carry CSS-to-physical DPR consistency into 11.0f. Evidence: group11/review-menu-*.log.

- 2026-11-17: 11.0f independently APPROVED after repairing child-observer/parent-scheduler ordering on fixed-to-dynamic changes. Initial connect/restart measures stable viewport after pending tab strip, shared initial/ongoing physical sizing applies DPR once, IPC validates transient dynamic overrides without persistence, fixed dimensions retained. Full frontend 516/default Rust306/test-support324 pass (four Rust skips), static/build checks pass; final ordering remediation 31 focused tests/tsc/lint pass. Native test-support executable built. Native acceptance pending. Evidence: group11/review-viewport-*.log.

- 2026-11-17: 11.0g independently APPROVED: persisted default-off session status-bar auto-hide, fixed overlay reveal without viewport changes, readable name/hostname/unknown labels, long-label truncation and local hover/focus input isolation. Settings saves publish confirmed values and retain prior state on failure. Independent 530 frontend / 308 default / 326 test-support Rust tests pass (four Rust skips), static/build checks pass with existing warnings. Native follow-up pending. Menu Escape integration correction independently APPROVED in ef7bd1c (17 focused tests), also awaiting native confirmation. Evidence: group11/review-status-*.log.

- 2026-11-17 user correction: status-bar reveal/hide must resize the session viewport to avoid overlap; the earlier overlay behavior is superseded. Reopened 11.0g for this bounded layout amendment. Native check had verified setting on, 28-pixel gain (Windows940x764), readable label, and corrected menu Escape. Keyboard traversal from the menu reaches the canvas before the reveal target; ensure a reachable local keyboard reveal path while preserving remote Tab. Test app stopped, CUA completed/reset, three test credentials removed.

- 2026-11-17: User-directed 11.0g flow amendment independently APPROVED. Revealed bar reserves 28 pixels in normal layout and existing resize handling sees the changed viewport; stationary edge trigger plus pointer/focus state avoids oscillation. Ctrl+Alt+Shift+S provides reachable local reveal without remote forwarding. 534 full frontend tests; independent61 focused tests/tsc/lint/Vite pass. No backend changes. Native resize confirmation pending. Evidence: group11/review-status-flow-*.log.




- 2026-11-17: 11.0h independently APPROVED; 15/16/24/32-bit bitmap requests and off/MPPC8K/64K/RDP6/RDP6.1 bulk compression persist per host, apply on reconnect, import and preserve independent presets. Independent536 frontend/310Rust4skip/docs0, actual app TypeScript/ESLint/Vite pass; implementer328 feature-tier4skip/fmt/clippy pass with two existing warnings. Native negotiation/performance not claimed. Use tsc -p tsconfig.app.json --noEmit (root references-only tsc is insufficient).

- 2026-11-17: 11.0i independently APPROVED: continuous1000ms pointer dwell, immediate focus, cancel on leave/route/mode/unmount. Independent539frontend/actualappTS/ESLint pass; Vite pass. Normal-flow sizing retained; native timing/resize acceptance pending.

- 2026-11-17: 11.0j independently APPROVED after two reproduced review findings were repaired: actual pinned CLIPRDR initial handshake, separate bootstrap/resync acknowledgements, and actor-side modifier preservation around delayed paste. Directional status-bar resync, connection epochs, bounded recovery/errors and input isolation implemented. Independent316Rust4skip/docs0/26focusedfrontend; prior unchanged frontend547/appTS/lint pass. Native acceptance pending; image/native local adapter follows11.0k.

- 2026-11-17: 11.0k independently APPROVED. Native text/image clipboard via compatible pinned plugin2.3.0, bounded PNG/DIB/DIBV5 conversion, lease/epoch/direction guards and stale-response recovery. Independent549frontend/339test-supportRust4skip/docs0/appTS/ESLint/fmt/productionClippy pass; default321 also passed. Storage checkpoint0.19GiB owned/31.47GiB shared. Native OS/real-host acceptance remains pending. Nonblocking follow-up: consolidate redundant image decoding while retaining trust-boundary validation.

- 2026-11-17: Partial 11.4 diagnostic remediation independently APPROVED: smoke handles bounded response-write failures and server termination, reports shutdown write failures truthfully, and separates connection gate from shrink/restore confirmation. Five regressions added; 31 focused baseline tests and docs/fmt/PowerShell checks pass. Real-host run group11/resize-diagnostics/real-hosts/smoke-20261117-175310: all three connection/first-frame/shutdown gates and redaction pass; host3 IronRDP confirms1280x720 then3180x1332, host1 Windows and host2 FreeRDP still time out with no session-end/write/process error. Full11.4 remains open; next compare bounded native app behavior against smoke. Computer Use stayed off. No product protocol change.

- [x] 11.0l Add verified-update integration and compact Settings controls with automatic startup checking, manual retry, explicit install and active-session protection. _Boundary: updater plugin, commands, state/UI and tests. Requirements:15.2,15.3,15.4. User-directed amendment approved2026-11-17._
- [x] 11.0m Build/publish per-user NSIS installer and signed preview update feed, preserving embedded assets/application identity and documenting signing limitations. _Boundary: release configuration/scripts/artifacts. Depends:11.0l. Requirements:15.1,15.3,15.5. User-directed amendment approved2026-11-17._

- 2026-11-17: 11.0m Windows x64 installer/feed delivery verified. Published v0.1.0-alpha.3 (source b568fa1), per-user NSIS install exits 0; installed app reaches embedded tauri.localhost dashboard and displays alpha.3 in Updates/About. Startup and manual checks against published preview-channel/latest.json both report latest preview. GitHub installer SHA256 matches local 205ac1f4d66270b47d8cb19a2af0817957c14573cbafdeedeae4e72371792391. Installer/feed scripts independently approved. Full upgrade-and-restart acceptance remains pending a subsequent version. Other target builds run in Actions 44143737759; no non-Windows runtime testing is authorized or claimed. Computer Use closed/reset. Windows publisher signature is absent, so SmartScreen/browser warnings remain possible. No mandatory build/storage guard.

- [x] 11.0n Replace misleading local-frame bandwidth with measured per-session transport Rx/Tx rates and observed graphics diagnostics in existing compact performance UI. _Boundary: transport instrumentation, actor metrics, session wire/UI and tests. Requirements:16.1-16.3. User-directed amendment approved2026-11-17._
- [x] 11.0o Research lossless coding bandwidth optimization and opt-in 5-minute idle/60-second preview mode against pinned upstream APIs; document evidence, limitations and implementation plan. _Boundary: research/spec notes only. Requirements:16.4. User-directed amendment approved2026-11-17._
- 2026-11-17: 11.0o independently APPROVED. Pinned connector capability flags, public upstream suppress/resume encoder and refresh helper, RAM/persistent cache interfaces verified. Research distinguishes local buffering from network savings and no-AVC from lossless; idle implementation and measured real-host savings are not claimed. Evidence: bandwidth-research.md. Sharp text immediately during typing/scrolling is the user acceptance criterion; no H.264 fallback.
- 2026-11-17: 11.0n independently APPROVED: successful TCP payload Rx/Tx below TLS, periodic elapsed-time rates, reconnect resets and fresh trailing totals; frontend local frame updates cannot replace network measurements. Existing HUD shows directional rates/totals and observed channel/codec or Unknown. Independent558frontend/352test-supportRust4existing skips/docs0/appTS/lint/fmt/productionClippy pass. Evidence D:/BuildArtifacts/drift-rdp/bandwidth/review-*.log. Source-only: no native host measurement or installer delivery claimed; alpha.3 installed artifact unchanged. No H.264, idle mode, FPS cap, profile or credential change.
