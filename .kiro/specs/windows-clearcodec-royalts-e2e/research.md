> Historical discovery log: the amendment below supersedes earlier macOS driver, sequencing, migration-failure and cross-library-move assumptions.

# Gap Analysis — windows-clearcodec-royalts-e2e

Generated: 2026-11-11 · Method: codebase investigation (main context + explore subagent) + external dependency research (docs.rs, crates.io, IronRDP GitHub issues) · Based on requirements.md (Req 1–13, not yet user-approved).

## 1. Current State — Reusable Assets

| Area | Assets | State |
|---|---|---|
| RDP session | `rdp/session.rs` (SessionActor, 1162 lines), `rdp/client.rs` (connector, TLS, NLA) | Working; ironrdp 0.14 facade, ironrdp-graphics **0.7.0** (locked) |
| Frame pipeline | `renderer/shared_frame.rs` (double-buffer + condvar), `renderer/gpu.rs` (wgpu surface on main window), `rdp/frame_transport.rs` (IPC packet protocol kinds 1/2/3), `lib.rs` L51–122 render thread | Working; visible path is IPC→webview canvas; GPU surface dormant (renders behind webview) |
| H.264 transport (to remove) | `renderer/h264_encoder.rs` (openh264), `session.rs` `try_send_h264_frame` L245–272 + call sites L280/L624–631, `frame_transport.rs` kind 3, `src/lib/frame-protocol.ts` `tryParseH264Packet`, `src/lib/h264-decoder.ts` (WebCodecs) | Isolated; no RDP-protocol coupling — removal is mechanical |
| UI shell | `layout/Sidebar.tsx` (flat 2-item nav, 264 lines), `session/SessionTabs.tsx`, `connections/*` (grid/card/form/quick-connect), `App.tsx` routes | Working; superseded by Req 10/11 |
| State/persistence | `stores/connectionStore.ts`, `types.ts` ↔ `store/connections.rs` (flat JSON array, camelCase serde), `store/credentials.rs` (keyring, 2 ignored tests) | Working; flat model — no folder concept |
| Platform | `main.rs` windows_subsystem attr, `icons/icon.ico`, Titlebar uses `platform()` from tauri-plugin-os, `useKeyboard.ts` uses `isMod = metaKey \|\| ctrlKey` | Partial Windows readiness |
| CI | `.github/workflows/ci.yml` — tsc/vitest/check/test/fmt/clippy + build matrix (mac aarch64/x64, linux x64) | No windows runner |
| Tests | 29 vitest (types, connectionStore, input-mapper 18, frame-protocol 1 — no dirty-rects/h264 tests), 22 Rust tests (network 5, display 5, frame_transport 1, connections store 9, credentials 2 ignored) | Unit tier only; no E2E |
| Misc | `native-app/` (3-line hello-world, no deps — dead scaffold), mock frame loop `session.rs` L940–995 (basis for E2E mock mode) | Usable |

Known defects relevant to delta: **(D1)** `SessionTabs.tsx` navigates `/session/${sessionId}` but `SessionView.tsx` L128–130 treats the route param as `connectionId`; **(D2)** `connectionTimeout` default TS 10000 vs Rust 30000.

## 2. Requirement-to-Asset Map

| Req | Existing coverage | Gap |
|---|---|---|
| 1–3, 5, 6 (baseline) | Implemented | None (regression baseline only) |
| 4 Rendering (modified) | Bitmap packet path (kinds 1/2) complete | H264 kind-3 removal; EGFX frame source (see Req 9) |
| 7 Platform/CI (modified) | mac+linux matrix | Windows matrix entries — see Req 8 |
| 8 Windows | `windows_subsystem` attr, icon.ico, isMod shortcuts, platform() titlebar | **Missing**: keyring `windows-native` feature; `client.rs` L126–127 platform `UNSPECIFIED` → `WINDOWS`; CI windows job + `x86_64-pc-windows-msvc` target + NSIS/MSI artifact (bundle config block absent); shortcut *labels* hardcoded "Cmd"; `titleBarStyle: Overlay` is macOS-only (verify Windows behavior) |
| 9 ClearCodec | IronRDP stack present; fast-path bitmap route working | **Missing (large)**: locked ironrdp-graphics 0.7.0 predates ClearCodec (0.9.0); no `ironrdp-egfx` dependency; connector never advertises EGFX; see §3 |
| 10 Tree sidebar | connectionStore CRUD, search, tags, status in SessionInfo | **Missing**: folder entity + tree data model + migration; tree UI components (expand/collapse, context menus, DnD, per-row status, filter-over-tree); collapse-rail behavior |
| 11 Document tabs | SessionTabs (inside SessionView), sessionStore sessions Map | **Missing**: persistent global tab bar across routes; route unification on sessionId (fixes D1); start view when no tabs |
| 12 E2E | mock frame loop exists (unreachable from UI?); vitest/cargo tiers | **Missing**: driver harness (tauri-driver etc.), tri-platform CI wiring, mock-session connection mode, E2E scenarios |
| 13 TDD | vitest + cargo test infra | **Missing**: process adoption; per-tier single commands documented; coverage of frame-protocol dirty rects etc. |

## 3. Deep-Dive: ClearCodec Feasibility (Req 9) — Highest Risk Item

Findings from IronRDP upstream (docs.rs, issues #1174/#1175/#1464, CHANGELOGs):

1. **ClearCodec decode exists**: `ironrdp-graphics` ≥ 0.9.0 (`clearcodec` module, `ClearCodecDecoder`, BGRA out, glyph/V-bar caches). Added 2026 (issue #1174, dispatch #1175 merged 2026-07-30).
2. **ClearCodec is an EGFX codec** (MS-RDPEGFX 2.2.4.1) — it arrives over the Graphics Pipeline dynamic virtual channel, **not** the fast-path bitmap route Drift currently uses. Enabling ClearCodec means enabling client-side EGFX.
3. **Client EGFX is now wired upstream** (tracking issue #1464, closed 2026-08-30): surface compositor (#1460), session-side render drain (#1461), native-client DVC registration (#1462), connector `RNS_UD_CS_SUPPORT_DYNVC_GFX_PROTOCOL` flag (#1237, merged 2026-08-30), Progressive dispatch (#1443), `AVC_DISABLED` preservation (#1490).
4. **"No H.264" maps cleanly**: `GraphicsPipelineClient::new(handler, h264_decoder: None)` — ClearCodec/Planar/Uncompressed/Progressive decode with no external decoder; AVC frames are logged and skipped; caps advertise `AVC_DISABLED`. We must simply not provide an H.264 decoder and strip openh264.
5. **Version reality**: Drift locks ironrdp 0.14 / ironrdp-graphics 0.7.0. The complete client EGFX stack landed **after** ironrdp 0.17.0 (2026-07-10); docs.rs lists ironrdp-egfx 0.5.0 as latest. **Constraint**: a major IronRDP upgrade is mandatory; exact facade version containing all pieces (or git pin) must be pinned in design. **Unknown**: whether a released ironrdp facade ≥ 0.17.1 includes #1461/#1462/#1237, or whether Drift must depend on git rev or re-implement the wiring in `SessionActor` (upstream wiring targets `ironrdp-client`, which Drift does not use — Drift has its own actor around `ActiveStage`).
6. **Breaking changes en route** (affect `client.rs`/`session.rs` rewrite): #1435 removed connector dep from session + new `ActiveStageBuilder`/`ConnectionActivationFactory`; #1377 typed DVC accessors (`get_dvc::<T>()`, `DvcClientProcessor`); #1389/#1391 client Config/PropertySet refactor; #1465 bandwidth-measure fix needed for FreeRDP-based servers (grd).
7. **Negotiation correctness rule** (upstream #1464 item 5): advertise only decodable codecs — caps = ClearCodec + Planar + Uncompressed (+Progressive), AVC disabled, `WANT_32_BPP_SESSION` set with the GFX flag. Feeds Req 9.1/9.3.
8. **Fallback**: if server lacks EGFX (older Windows, some servers), IronRDP stays on fast-path bitmaps — Req 9.3 satisfied structurally; E2E/manual verification per server class (Research).

## 4. Deep-Dive: Windows (Req 8)

- `keyring` 3: add `windows-native` feature (Windows Credential Manager). CredentialStore code is platform-agnostic; only Cargo.toml changes + CI coverage.
- `client.rs`: add `#[cfg(target_os = "windows")] platform: MajorPlatformType::WINDOWS` (currently falls to UNSPECIFIED).
- CI: new `windows-latest` entries for check/test/clippy + build matrix `x86_64-pc-windows-msvc`; artifact globs add `*.exe`/`*.msi`/NSIS. Tauri bundle config: add `windows` block (nsis/wix) if defaults insufficient; WebView2 bootstrapper default.
- wgpu 24 → DX12 backend on Windows; GpuRenderer uses raw-window-handle — verify surface creation on HWND (Research: smoke test).
- Shortcuts: handlers already cross-platform (`isMod`); only display labels ("Cmd+K") need platform awareness via `platform()`.
- `titleBarStyle: "Overlay"` is ignored on Windows (standard decorations) — acceptable baseline; custom chrome out of scope.
- MSRV: rust-version 1.77.2 — verify new IronRDP stack's MSRV (Research; upstream may require newer).

## 5. Deep-Dive: GUI Refactor (Req 10–11)

- **Data model options**: (A) add `folderId: Option<String>` to ConnectionConfig + `Folder` entity in connections.json array — minimal migration, flat store logic mostly reusable; (B) nested `children` tree — prettier JSON, harder diff/moves; (C) Royal TS-style discriminated "documents" model — most faithful, largest change. Recommendation leans A for delta (design decides).
- Migration: existing flat array → folders default root; version field or presence-based detection; test with golden JSON files.
- UI: replace `Sidebar.tsx` nav with tree (new `ConnectionTree.tsx` + `TreeNode.tsx`, keep framer-motion/shadcn/Radix patterns); context menus via Radix ContextMenu; DnD via HTML5 dnd or dnd-kit (new dep — design decision); status icons subscribe to sessionStore.
- Tabs: hoist SessionTabs to App-level persistent bar keyed by sessionId; unify routes on sessionId (D1 fix); start view replaces ConnectionGrid home.
- ConnectionGrid/card UX (latency-on-mount etc.) is superseded; salvage: ConnectionForm (modal), QuickConnect (Cmd+K) remain.

## 6. Deep-Dive: E2E + TDD (Req 12–13)

- **Driver options**: (A) `tauri-driver` + WebdriverIO — official path; Windows→msedgedriver (WebView2), macOS→safaridriver, Linux→WebKitGTK driver. (B) Playwright attaching via CDP — WebView2-only realistically, not tri-platform. (C) Vitest browser-mode against dev server — cannot exercise Tauri IPC. Leans A; tri-OS CI stability (safaridriver permissions, headed runners) is the main unknown (Research).
- **Mock session mode**: `run_mock_frame_loop` exists but is internal; expose as hidden connection flag (e.g. `mock: true` or host `mock://`) so E2E can create a "connection" that renders synthetic frames + status transitions without a server. Also enables Req 12.4 (assert only kinds 1/2 on the wire).
- **TDD**: per-tier single commands (`npm run test:e2e`, existing vitest/cargo); task list must sequence test-first; frame-protocol tests extended (dirty rects, unknown kind, kind-3 removal).

## 7. Implementation Approach Options (per area)

| Area | Option A: Extend | Option B: New components | Option C: Hybrid (recommended) |
|---|---|---|---|
| ClearCodec/EGFX | — | Reimplement EGFX wiring in SessionActor on current 0.14 | **Upgrade IronRDP + adapt SessionActor to upstream wiring patterns**; new `rdp/egfx.rs` module for DVC registration + compositor drain into SharedFrame/IPC |
| Windows | cfg sprinkles in place | — | Mostly A: feature flags + CI + bundle config in place |
| Tree sidebar | Bloat Sidebar.tsx | All-new tree components + store slice | New `ConnectionTree` components + extend connectionStore model (A for store, B for UI) |
| Tabs | Move SessionTabs to App | New DocumentTabs | Adapt existing SessionTabs → App-level (extend) + route unification |
| E2E | — | New harness dir + workflow job | New harness; extend connection model with mock flag |

## 8. Effort & Risk

| Req | Effort | Risk | Justification |
|---|---|---|---|
| 8 Windows | M | Medium | Known patterns; DX12/wgpu + WebView2 CI quirks unverified here |
| 9 ClearCodec | **XL** | **High** | Major IronRDP upgrade (0.14→≥0.17+, breaking APIs), EGFX wiring in custom actor, codec negotiation correctness, frame pipeline rework |
| 10 Tree | L | Medium | Data migration + DnD + new UI surface |
| 11 Tabs | M | Low | Layout refactor; route fix is contained |
| 12 E2E | L | Medium | Tri-OS driver matrix; mock mode plumbing |
| 13 TDD | S | Low | Process discipline |

## 9. Research Items for Design Phase

1. Pin exact IronRDP release (≥ facade containing #1461/#1462/#1237) vs git dependency; check MSRV vs 1.77.2 and adapter effort for `ActiveStageBuilder`/typed DVC API in Drift's actor.
2. Whether Drift reuses upstream `ironrdp-client` graphics-DVC registration or hand-wires `DrdynvcClient::with_dynamic_channel` in `SessionActor` (Drift does not use ironrdp-client).
3. EGFX caps set to advertise (ClearCodec+Planar+Uncompressed+Progressive? AVC_DISABLED interplay with Windows servers — will Windows fall back to EGFX non-AVC or to fast-path?) + behavior vs real Windows Server targets (manual interop plan).
4. wgpu-on-HWND smoke test for GpuRenderer (DX12) — GPU surface stays dormant but must not panic.
5. tauri-driver tri-OS CI: safaridriver permissions on macOS runners, msedgedriver version matching WebView2 runtime, headed vs headless (Xvfb) on Linux.
6. DnD choice for tree (native HTML5 vs dnd-kit dep).
7. Frame-packet protocol versioning while removing kind 3 (frontend/backend contract test).
8. Migration test strategy for connections.json (golden files, version field?).

## 10. Recommendations (non-binding, for design)

- Sequence delta as: **(1)** Windows platform (independent, unlocks CI tri-OS early), **(2)** E2E harness + mock mode (regression safety net), **(3)** tree/tabs GUI refactor (covered by new E2E), **(4)** IronRDP upgrade + ClearCodec/EGFX last (highest risk, benefits from safety net).
- Keep fast-path bitmap route intact as the non-EGFX fallback (Req 9.3) — EGFX is additive.
- Strip openh264 early (mechanical, testable via kind-3 absence) independent of EGFX work.

---

# Gap Analysis Addendum — Amended Requirements 10/11 (2026-11-11)

Trigger: requirements amendment after user review — Req 10 now includes **multi-document connection libraries** (top-level tree nodes, create/rename/delete) and a **folder/library dashboard** (main panel lists contained connections incl. nested, live status, double-click connect); Req 11.5 shows the selected node's dashboard when no tab is open. Windows/ClearCodec/E2E/TDD analysis in §3–4, 6–9 is unaffected and not repeated.

## A1. Multi-Document Libraries — Gap

**Assets that survive**: UUID v4 connection ids (`connections.rs` L114 — unique across libraries, so keyring `account = connection id` needs no change); CRUD validation/import logic; camelCase serde contract; `ConnectionStore` mutex pattern.

**Missing**:

| Layer | Gap |
|---|---|
| Data model | No `Library` entity; `connections.json` is one flat array; no folder→library ownership |
| Rust store | `ConnectionStore` (HashMap + single file) → needs library-aware layout + migration marker |
| IPC | No `create_library` / `rename_library` / `delete_library` commands; existing CRUD commands not library-scoped |
| TS | `lib/tauri.ts` wrappers + `connectionStore` assume flat list; no tree/uiState store slice for selection |
| UI | Library tree nodes, library-level context menu, cross-library DnD, per-library filter scope |

**Data-model options (updates §5; the "leans A" note there is superseded)**:

- **A′ — single file, library+folder ids**: `connections.json` becomes `{ libraries: [...], connections: [{..., libraryId, folderId}] }`. Minimal migration; one file to back up; no per-library sharing.
- **B′ — per-library files**: each library its own JSON file + small index. Royal TS-faithful (a library ≈ an `.rtsz` document); enables later export/share/sync per library; more store/IO rework.
- **C′ — discriminated document tree**: one tree of nodes (`library | folder | connection`). Most uniform for tree ops (moves become node reparenting); largest refactor of store + TS types.

All three satisfy Req 10; A′ is lowest risk, B′ best matches Royal TS mental model and future sharing, C′ cleanest for deep nesting. Design decision — new Research item A-1.

**Migration**: flat array v1 → default library at root (Req 10.10). Presence-of-`libraries` key or explicit `version` field; golden-file tests (extends §9 item 8). Delete-library must warn about contained connections (destructive cascade — design defines confirmation UX; keyring cleanup best-effort per connection).

## A2. Folder Dashboard — Gap

**Reusable**: `ConnectionCard` patterns (status color, latency-on-mount, accent, entry animations), grid CSS, sessionStore live status subscription, `PerformanceHUD` thresholds n/a here.

**Missing**: dashboard view component bound to the selected tree node; selection state store (new uiState slice: selectedNodeId, expandedNodes, collapse state — collapse pref currently in settingsStore, keep); double-click handler shared by tree + dashboard; start view (quick connect + tree pointer) when nothing selected; **QuickConnect must search across all libraries** (currently lists `connectionStore.connections` flat — works but must aggregate libraries once A′/B′/C′ lands).

**Route impact**: main area now hosts tabs ∪ dashboard ∪ start view; complements Req 11 route unification (sessionId). Suggest route-per-view (e.g. `/library/:id`, `/folder/:id`, `/session/:sessionId`) — design decision, not gap.

## A3. Updated Effort & Risk

| Req | Was | Now | Why |
|---|---|---|---|
| 10 Tree+libraries | L / Medium | **L–XL / Medium-High** | Library entity + migration + cross-library DnD; C′ pushes toward XL |
| 11 Tabs+dashboard | M / Low | **M / Low-Medium** | Dashboard/start view added; route unification unchanged |

Sequencing recommendation (§10) unchanged; note the GUI refactor now carries more of the delta's weight if C′ is chosen.

## A4. Added Research Items for Design

- **A-1**: Library storage layout — A′ vs B′ vs C′ (drives store, migration, IPC, and future sharing).
- **A-2**: Delete-library cascade UX + keyring cleanup contract.
- **A-3**: QuickConnect + filter scope across libraries (aggregate query shape).
- **A-4**: E2E scenario list should explicitly cover library create/rename/delete + dashboard selection and double-click connect (Req 12.3 "sidebar tree interactions" subsumes it; enumerate in design/tests).
- **A-5**: Royal TS "open/close document" semantics deliberately not required (Req 10.2 stops at create/rename/delete) — confirm acceptable.

---

# Design Discovery & Synthesis Log (2026-11-11)

## Discovery Additions (version pinning — resolves Research Item 1)

- `ironrdp` facade latest = **0.17.0** (2026-07-10, MSRV Rust 1.89, edition 2024). Its feature set has **no `egfx` feature**, and it predates the 2026-08-27/30 end-to-end EGFX merges (#1461/#1462/#1237).
- `ironrdp-egfx` latest = **0.5.0** (2026-10-27, edition 2024). Features: `openh264`, `openh264-bundled`, `openh264-libloading` — **all optional, off by default**; ClearCodec/Planar/Uncompressed/RFX-Progressive decode with no features enabled. Depends on the pdu/graphics ≥ 0.9 generation.
- **Decision**: depend on IronRDP component crates directly (facade README explicitly supports this); pin the set aligned with egfx 0.5.0; never enable openh264 features; bump `rust-version` 1.77.2 → 1.89. Exact pin + `ActiveStageBuilder`/typed-DVC adaptation = **Task 0 alignment spike** (design.md Risks).

## Synthesis Outcomes

**Generalizations**
- Platform matrix (Req 7 + 8) treated as one capability-matrix concern: cfg flags, CI jobs, bundle config, shortcut labels.
- Libraries/folders/connections unified as `NodeId`-keyed tree nodes → single DnD/context/dashboard/filter implementation (Req 10).
- Test tiers generalized into single-command pyramid contract (`test:unit`, `test:e2e`, `cargo test`) feeding tri-OS CI (Req 12 + 13).

**Build vs Adopt**
- ADOPT: `ironrdp-egfx` 0.5 (EGFX client + ClearCodec decode) — building this is out of the question; upstream wiring patterns guide `rdp/egfx.rs`.
- ADOPT: WebdriverIO + tauri-driver for E2E; dnd-kit for tree DnD (fallback native HTML5 if React 19 peer conflict).
- BUILD: CodecPolicy (caps single-source-of-truth), LibraryStore v2 + migration, ConnectionTree/Dashboard/DocumentTabs UI, mock-session exposure, frame-kind spy. No existing crate covers these app-specific seams.

**Simplifications**
- Data model = option **A′** (single `connections.json` v2 with `libraries`/`folders`/`connections`); per-library files (B′) and discriminated tree (C′) rejected for now — B′ is a recorded Revalidation Trigger, not lost.
- SessionActor preserved (not rewritten to ironrdp-client); EGFX is an additive frame source; frame packet kinds 1/2 contract frozen so frontend changes stay minimal.
- DashboardCard drops ConnectionCard's latency-on-mount probing (status-first, cheaper).

## Consulted Guidance
- `design-principles.md` (boundary-first, dependency direction, mermaid rules), `design-synthesis.md`, `design-review-gate.md`, design template; steering product/tech/structure (authored this session from assessment).

## Review amendment (2026-11-11)
- Direct tauri-driver/safaridriver does not provide the macOS WKWebView path. Prove the WebdriverIO Tauri service embedded macOS server before upgrading IronRDP. Source: https://tauri.app/develop/tests/webdriver/
- ClearType requires a separate font-smoothing preference/request. ClearCodec does not establish font smoothing. Source: https://learn.microsoft.com/en-us/windows-server/administration/performance-tuning/role/remote-desktop/session-hosts
- Foundation now precedes protocol changes: baseline characterization and tri-OS cold launch. Test presence is not full baseline coverage.
- Cross-library moves are atomic and supported. Migration/load failure enters write-blocked recovery.
- Task numbers shifted by one to insert foundation group 1; historical task references above are superseded by tasks.md.
- Requirements/design approvals are reset for amended content; readiness remains false. No passing runtime tests are claimed.


## Effort-control, nextest and Dagger decisions
The user selected cargo-nextest and Dagger-based Actions with native Windows shell execution. The new task plan supersedes previous task-number references; no implementation is marked complete.
- Reuse existing tempfile first for replacement; its persist API and sync limitations need platform tests: https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html
- Alternative atomic-write-file is not selected by default: https://docs.rs/atomic-write-file/latest/atomic_write_file/
- Tokio test-util offers paused time on current-thread test runtimes, not std::time control: https://docs.rs/tokio/latest/tokio/time/fn.pause.html
- Nextest runs native Rust tests but needs separate doctests: https://nexte.st/ ; Windows overhead should be measured: https://nexte.st/docs/installation/windows/
- Upstream currently advises nextest 0.9.150+; pin an applicable release when implementing, rather than float latest.
- Dagger engine selection supports Podman; native engine connectivity remains a local feasibility gate: https://docs.dagger.io/reference/cli/
- Actions integration remains a thin bridge: https://github.com/dagger/dagger-for-github
- IronRDP client docs demonstrate upstream compositor ownership: https://docs.rs/ironrdp-egfx/latest/ironrdp_egfx/client/index.html . Fetching the proposed 0.5.0 docs failed during planning, so version compatibility is unverified until the compile probe.
- Local command discovery found nextest.exe plus PowerShell wrappers for Dagger and Podman. Wrapper delegates were not executed or fully audited; existence is not proof of usable native engine connectivity.

## Dagger-Podman connectivity spike (task 1.3, 2026-11-11) - PROVEN

Outcome: native Windows Dagger-to-Podman connectivity proven within the 3h cap; no blocking result. Full evidence tree (disposable BuildArtifacts, not committed): `D:\BuildArtifacts\drift-rdp\dagger-bootstrap\` (EVIDENCE.md, logs 01-12 incl. podman inventory, proof artifacts, scratch TS module).

- Engine: existing WSL-backed Podman machine `podman-machine-default` (Fedora 44, kernel 6.18.58.1-microsoft-standard-WSL2, server 5.8.9) via native client 5.8.3; rootful default connection `ssh://root@127.0.0.1:62042/run/podman/podman.sock`. No direct WSL commands, no new VM, no Compose.
- Dagger pin: CLI v0.21.11 + engine `registry.dagger.io/engine:v0.21.11`. Discovery is NOT DOCKER_HOST (unset): user launcher `C:\Users\mariu\.local\bin\dagger.cmd` -> `D:\Source\PowerShell\scripts\Invoke-Dagger.ps1` sets `_EXPERIMENTAL_DAGGER_RUNNER_HOST=image+podman://registry.dagger.io/engine:v0.21.11`. Task 1.4 must encode this env in Invoke-CI/workflow instead of relying on the personal launcher. Pins also live in `developer-tools.psd1`.
- Proof: `dagger query` core API ran alpine:3.21 withExec, exported artifact to scoped dir (exit 0); cache-bypass fresh-exec marker proves real execution. Scratch TS SDK module `dagger call proof export` also exit 0. Independently reproduced by the task-1.3 reviewer.
- Caching finding for 2.7: identical withExec inputs replay cached stdout (content-addressed); cache-parity and E2E determinism work must account for it.
- Module layout: dagger 0.21 `dagger init` defaults may place sources elsewhere; 1.4 must pin `sourceSubpath` explicitly (design file plan reserves `.dagger/`).
- State left: podman machine running (as found); engine container `dagger-engine-v0.21.11` running (was stopped on arrival).

## Native Windows cold-launch spike (task 2.3, 2026-11-11) - PROVEN

Outcome: built-app native Windows cold launch on the old stack (HEAD 5d1c1bb) PASSES with ZERO minimum compile fixes; no blocking prerequisite for the upgrade. This is native Windows validation only — no container/WSL involvement, and it does not speak to macOS/Linux. Full evidence tree (disposable BuildArtifacts, not committed): `D:\BuildArtifacts\drift-rdp\windows-cold-launch\` (EVIDENCE.md, 00-environment, 01-red-cargo-check, 02-tauri-build, 03-msedgedriver-pin, Invoke-ColdLaunch{,2}.ps1, run-1/2/3 with screenshots + per-run reports, pinned-driver\). Actual work time ~23 min of the 3h cap.

- Environment (verified, not assumed): Windows 11 Pro build 26200 x64; rustc/cargo 1.98.1 (rustup default host x86_64-pc-windows-msvc); VS 17.14.41 (VC.Tools.x86.x64); node 24.22.1/npm 11.19.1; tauri-cli 2.10.1; Edge stable 157.0.4341.42 = WebView2 Evergreen runtime 157.0.4341.42 (HKLM EdgeUpdate pv). CARGO_TARGET_DIR=D:\BuildArtifacts\cargo-target honored.
- Compile/build: `cargo check` at HEAD with NO fixes exit 0 (1m31s). `npx tauri build` exit 0 — frontend tsc+vite OK, release compile 6m29s, `rdp-client.exe` (26.2 MB) plus bundles `Drift_0.1.0_x64_en-US.msi` (WiX 3.14.1, auto-downloaded) and `Drift_0.1.0_x64-setup.exe` (NSIS 3.11, auto-downloaded). Why no keyring fix was needed: keyring 3.6.3 ships `windows-native` in its DEFAULT features, so the Windows Credential Manager backend compiles although our manifest lists only apple-native/linux-native (design.md's explicit `windows-native` addition remains a new-stack clarity change). `src/rdp/client.rs` already carries a not(macos/linux)->UNSPECIFIED platform fallback (8.3 stays new-stack work). Cargo.toml byte-identical to HEAD after the spike (blob-hash match; transient `M` git status was a CRLF stat artifact).
- Cold launch (runs 2+3, final harness, both PASS identically): launch the built exe with APPDATA/LOCALAPPDATA/TEMP/TMP/WEBVIEW2_USER_DATA_FOLDER overridden into the evidence tree → window title 'Drift' 1216x839 visible (EnumWindows by pid; Process.MainWindowHandle can be a 16x16 tao helper with empty title — v1 harness lesson), 6 msedgewebview2.exe children on the ISOLATED user-data folder (WebView2's own log: CurrentProfilePath=...run-N\webview2\EBWebView\Default; 169 profile files landed only there), screenshot 1216x839 with 128 distinct sampled colors (real painted UI), app log "GPU renderer created: adapter=AMD Radeon(TM) 890M Graphics, format=Rgba8UnormSrgb, present_mode=Fifo, size=1200x800" + "[GPU] First frame rendered successfully!" (wgpu picked VULKAN on this host — wgpu_hal::vulkan warnings), clean shutdown WM_CLOSE → exit code 0 in <1s.
- Isolation (12.7): WebView2 state fully isolated via WEBVIEW2_USER_DATA_FOLDER (wry 0.54.3 passes an empty userDataFolder so the loader applies documented defaults incl. that env var — verified in wry src/webview2/mod.rs L287-345 and empirically). Credential Manager untouched every run (cmdkey diff 0 drift entries). CAVEAT for 2.5: Tauri app-data/log dirs resolve via Known Folder APIs and IGNORE APPDATA/LOCALAPPDATA env overrides — each run creates Roaming\com.drift.app (empty) + Local\com.drift.app\logs\Drift.log. Those were ABSENT at baseline (this profile has no real Drift data), so the harness treats com.drift.app as the run's own resource namespace and deletes exactly it afterwards; real profile verified RESTORED TO BASELINE after every run. True per-run app-data isolation needs an e2e-specific identifier (build-time config overlay) — decide in 2.5.
- Driver pin (12.8 Windows driver path): msedgedriver 157.0.4341.42 exact-matching the installed runtime, from `https://msedgedriver.microsoft.com/<pv>/edgedriver_win64.zip` (zip SHA256 33F2ACB2...7D43, exe SHA256 B95A8FDF...3A9A8; full hashes in 03-msedgedriver-pin.txt); `--version` self-reports the exact version; liveness proven via /status ready=True on a scratch port. ALL legacy channels probed dead (msedgedriver.azureedge.net and *cdn variants: DNS retired; msedgedriver.storage.googleapis.com: 404 incl. LATEST_STABLE and current Stable 157.0.4341.46; edgeupdates API no longer lists WebDriver artifacts).
- Drift risk for 2.5/2.6: Microsoft shipped Stable 157.0.4341.46 on 2026-11-11 13:55Z (0-day CVE-2026-125346/125350); this host's browser+runtime auto-update off .42 imminently. CI must resolve the driver version AT RUN TIME from the installed runtime's `HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` pv, download the exact match, and verify SHA256; a static version pin rots within days.
- Open items for 2.5/2.6 wiring: (1) driver resolution script per above; (2) app-data isolation decision (identifier overlay vs documented own-resource-cleanup contract); (3) WebdriverIO→WebView2 attach strategy (msedgedriver + WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS/--remote-debugging-port, or the Tauri wdio service's msedgedriver path) — NOT wired here, spike stopped at the pinned-binary + /status proof per boundary; (4) `bundle identifier com.drift.app ends with .app` warning — macOS-only concern, revisit if bundling issues appear.
- State left: no repo file changes (Cargo.toml blob-identical to HEAD); real user profile clean (com.drift.app dirs deleted after each run, verified baseline-equal); pinned driver zip+exe under the evidence tree; release exe/bundles in the shared cargo target dir; no drivers/services left running.

## Dagger Linux cold-launch spike (task 2.5, 2026-11-11) - PROVEN

Outcome: built-app Linux cold launch through the Dagger entry point PASSES on the old stack (HEAD 7da227e + task 2.5 wiring); no blocking prerequisite for the upgrade. `linux-e2e` is now an EXECUTABLE cold-launch CI gate (was: throw-only placeholder). These are Linux-container results and never substitute for the native Windows (2.3, PROVEN) or macOS (2.6, pending) validations. Full evidence tree (disposable BuildArtifacts, not committed): `D:\BuildArtifacts\drift-rdp\linux-cold-launch\` (EVIDENCE.md, 00-environment, 01-probes, 02-red-ct13, 03-live-run, 04-failure-path, 05-final-suite-green; the sabotaged staged source was deleted after evidence capture). Actual work ~1h40m of the 3h cap.

- Container environment (sub-goal 1): NO new system gap. The WebKitGTK runtime ships with the already-pinned `libwebkit2gtk-4.1-dev` (probe2: AppImage launcher → `rdp-client` → WebKitNetworkProcess + WebKitWebProcess). New pinned apt extras `tauriLinuxE2EAptPackages` in `.config/ci-versions.json` + `.dagger/src/pins.ts` (mirrored, CT13j): `xvfb` (headless display), `x11-utils` (xdpyinfo display-up proof + xwininfo window evidence), `xdotool` (window search + WM_DELETE_WINDOW close), `imagemagick` (`import` screenshots, `identify` distinct colors). Provisioned ONCE as a cached layer in `linuxBuildContainer` before the source mount, shared by linux-build and linux-e2e (linux-build pays ~40 MB it does not use; the once-per-image-pin layer was judged worth it over a per-source-change apt layer after the e2e chain). AppImage runs via `--appimage-extract-and-run` — no FUSE needed at run time (libfuse2 stays for the bundler). No dbus daemon, no GPU, no Vulkan ICDs required: wgpu renders through Mesa's llvmpipe software rasterizer that comes with the GTK stack ("GPU renderer created: adapter=llvmpipe (LLVM 15.0.6, 256 bits) ... [GPU] First frame rendered successfully!"); probe1's "Failed to initialize GTK" panic was a probe bug (DISPLAY not exported), not a stack gap.
- Isolated app-data (sub-goal 2, Req 12.7 — resolves the 2.3 handoff question): on Linux, Tauri/WebKitGTK DO respect XDG env overrides (unlike Windows Known Folder APIs): the harness scopes HOME, XDG_DATA_HOME, XDG_CONFIG_HOME, XDG_CACHE_HOME and XDG_RUNTIME_DIR (0700) into a per-run `/tmp/drift-e2e-home`; the app log confirms `App data directory: /tmp/drift-e2e-home/share/com.drift.app` and appdata-files.txt records exactly the files written (dconf + mesa_shader_cache + com.drift.app). No identifier overlay needed on Linux.
- Wiring (sub-goal 3): `linuxE2E` in `.dagger/src/index.ts` chains `scripts/linux/cold-launch.sh` (staged with the source; CR-stripped in-container as defense-in-depth) on top of the SAME verified bundle container as linux-build — new shared private method `linuxBundleBuildContainer()` (linuxBuild is now a thin wrapper; identical inputs replay the build layers from the engine cache, the 2.5 run never rebuilds what 2.2 cached). Harness: Xvfb 1280x800x24 → launch built AppImage → poll for the mapped `Drift` window (xdotool, exact title) → settle → screenshot + distinct-color count (blank/1-color fails: `screenshot-not-painted`) → xwininfo/process-tree/appdata evidence → graceful WM_DELETE_WINDOW close (SIGTERM/SIGKILL escalation recorded) → status JSON. Tier: `-DaggerEntry linux-e2e` now chains `export --path ...artifacts\linux-e2e` and writes the per-entry report incl. coldLaunchOk/Reason/ScreenshotExported/FailureScreenshotExported.
- Live run (sub-goal 3 evidence, 03-live-run): `pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier dagger -DaggerEntry linux-e2e` → exit 0 (whole tier 258s incl. first-time apt layer + rebuild; harness itself 7.3s); Drift window mapped after 3s, screenshot 1280x800 with 1347 distinct colors, shutdownPath graceful-close; exported under `D:\BuildArtifacts\drift-rdp\dagger\artifacts\linux-e2e\` (status JSON, cold-launch-screenshot.png, app/xvfb logs, window/process/appdata evidence).
- Fail-loudly contract (sub-goal 4, design decision): a FAILED cold launch does NOT throw in the module — the entry returns the evidence Directory precisely so the tier can export the failure screenshot BEFORE failing the job with exit 1 (a thrown dagger call cannot chain `export --path`, so module-side throwing would LOSE the failure evidence). The module still THROWS for prerequisite/evidence-loss failures (missing bundles/harness, no status JSON at all — CT13d), and the TIER reads the exported `cold-launch-status.json`: ok=false → exit 1 naming the harness reason + exported failure screenshot (CT13p/q/r, live demo 04-failure-path); ok=true without the screenshot → exit 1 (no fake pass); exit-0 dagger call with NO status → exit 1 (CT13n). CT11t is untouched: the `contract` entry still takes no --source and no export.
- Failure-path proof (04-failure-path): a sabotaged staged source (harness copy with `DRIFT_E2E_BREAK_LAUNCH=1` defaulted on → AppImage made non-executable → launch dies pre-window) run through the real entry: dagger call exits 0 returning evidence, export lands `cold-launch-status.json` (ok=false, reason drift-window-not-found) + `cold-launch-failure-screenshot.png` + app log (Permission denied); the tier gate on that status fails the job (proved functionally by CT13p/q/r through the exit-0 shim; the shim-free live sabotage demonstrates the module/export half). The seam is harness-only (documented in the script header); no app-code test hooks.
- Tests (sub-goal 5, RED-first): CT13a–CT13u added to scripts/Invoke-CI.Tests.ps1 — 19 assertions FAILED at HEAD before wiring (02-red-ct13 transcript: "176 passed, 19 failed"), all green after; CT12c/CT12d extraction targets updated to `linuxBundleBuildContainer` (the 2.2 build steps moved into the shared pipeline both entries call — same literals asserted, no gate weakened); the old throw-while-e2e/-absent guard is REPLACED by the real minimal run per the tasks.md Implementation Note ("linuxE2E must throw once e2e/ exists until 2.5 wires the actual run" — this task wires it; the full WebdriverIO suite remains later 12.x work and will extend the same entry). `npm run lint` green; full CT suite green (195 passed, 0 failed).
- Teardown observation for later groups: graceful close under bare Xvfb (no WM) exits code 1 with a GDK async X error at teardown (error_code 9, request 14 — resource destruction), while the window itself was mapped/painted/functional until close; Windows WM_CLOSE exits 0 (2.3). Not a launch blocker; a lightweight WM in the e2e image may clean it up when the WebdriverIO suite lands.
- Open items for the full WebdriverIO wiring (12.x groups): (1) WebKitGTK driver (`WebKitGTKDriver` binary) pin + WebdriverIO service config inside this same container pattern; (2) decide WM (none vs openbox) for teardown cleanliness + focus handling; (3) mock-session mode UI (12.2/12.3) once the suite drives real UI; (4) e2e/-suite directory + `npm run test:e2e` integration into this entry; (5) cold-launch duration on ephemeral CI runners (no engine cache) — record in the 2.7 parity gate; (6) Xvfb RANDR/resize behavior if E2E needs viewport changes (tauri window is fixed 1200x800 today).
- State left: repo changes only the task 2.5 files (module, pins, ci-versions.json, tier, CT tests, README, scripts/linux/cold-launch.sh); probe container `drift-probe2` + sabotage staged source under the evidence tree removed; dagger engine left running (as found); host cargo target untouched; no WSL/direct-engine commands used (native podman/dagger only).

## CI old→new gate parity record (task 2.7, 2026-11-11)

Outcome: retroactive one-for-one parity comparison between the pre-rework workflow (recovered verbatim via `git show 344fe41:.github/workflows/ci.yml`) and the task-2.6 thin workflow + `.dagger` module entries. EVERY old gate has a verified replacement: 12 rows, all PARITY or PARITY-WITH-DELTA, **0 GAP**. No remediation was required for parity; one non-blocking 2.6-review hardening was applied while re-verifying (see below). Requirements 7.1, 7.2, 7.3, 12.8; design.md "CI ownership and reuse decisions" and "Testing Strategy → Delivery gates and brownfield coverage".

- **Deviation disclosure (recorded, not hidden):** design.md "CI ownership and reuse decisions" says "Move existing checks one-for-one and verify gate/artifact parity before removing duplicate Linux workflow logic"; task 2.6 removed the old Linux shell jobs during the workflow rework, before this comparison ran (disclosed in the 2.6 review and tasks.md Implementation Notes). This section IS the parity comparison, executed retroactively against the git-recovered pre-rework workflow — the comparison is against exact old text, not memory.
- **Method (exact sources compared today):** old workflow `git show 344fe41:.github/workflows/ci.yml` (7 jobs: typecheck, frontend-test, rust-check, rust-test, rust-fmt, rust-clippy, 3-target build matrix) vs new `.github/workflows/ci.yml` (dagger-frontend, dagger-rust-checks, dagger-linux-build, dagger-linux-e2e, windows-native, macos-bundle) + `.dagger/src/index.ts` / `.dagger/src/pins.ts` command inventory + `scripts/Invoke-CI.ps1` tier twins. The old apt dependency list was compared package-for-package with `pins.tauriLinuxAptPackages` — identical 7-package set (libwebkit2gtk-4.1-dev, libappindicator3-dev, librsvg2-dev, patchelf, libdbus-1-dev, libssl-dev, pkg-config).

### Parity table (old gate → new home; every row cites the replacement's exact command)

| # | Old gate @344fe41 (runner; exact commands) | Old inventory → artifacts | New home (job + entry; exact commands) | New artifacts | Disposition |
|---|---|---|---|---|---|
| 1 | typecheck (ubuntu-latest, node 22, npm ci) `npx tsc --noEmit` | repo TS sources → none | dagger-frontend → `frontend` entry: `npm ci`, `npm run lint`, `npx tsc --noEmit` in pinned node:22.23.4-bookworm container | vitest JUnit + failure evidence upload | PARITY-WITH-DELTA: exact tsc command kept; lint + build + dist-existence check and JUnit are additions (old workflow had no lint gate at all); pinned image replaces floating node-22 |
| 2 | frontend-test (ubuntu-latest, node 22) `npx vitest run` | full vitest suite → none (no JUnit) | same `frontend` entry: `npx vitest run --reporter=default --reporter=junit --outputFile.junit=/test-results/vitest-junit.xml` | vitest-junit.xml exported + uploaded | PARITY-WITH-DELTA: same run command/suite (JUnit reporter added); durable JUnit evidence 25 tests/0 failures (2.1 export; fresh 2.7 host inventory run identical: 25/0 in 450 ms — native host count for inventory only; the CI gate is the Linux-container run) |
| 3 | rust-check (ubuntu-22.04, @stable, apt deps, wd src-tauri) `cargo check` | workspace crates → none | dagger-rust-checks → `rust-checks` entry: `cargo check --manifest-path src-tauri/Cargo.toml` (repo-root invocation ≡ old working-directory form) | rust JUnit + failure evidence upload | PARITY: identical command semantics + identical apt set; pinned rust:1.98.1-bookworm image replaces floating @stable toolchain |
| 4 | rust-test (ubuntu-22.04) `cargo test` | 22 tests incl. 2 ignored + doctests → none | `rust-checks` entry: `cargo-nextest nextest run --config-file .config/nextest.toml --profile ci --manifest-path src-tauri/Cargo.toml --no-tests fail` + `cargo test --doc --manifest-path src-tauri/Cargo.toml` | rust-junit.xml (nextest ci profile) | PARITY-WITH-DELTA: nextest replaces `cargo test` for unit/integration (same binaries; durable JUnit 20 tests/0 failures = 22 minus the 2 config-skipped keyring tests, matching the old 2 ignored) and doctests stay via `cargo test --doc` — mandated by Req 7.1's explicit naming of both commands; also adds JUnit + fail-on-zero |
| 5 | rust-fmt (ubuntu-latest, rustfmt) `cargo fmt --check` | formatting → none | `rust-checks` entry: `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | (same entry's uploads) | PARITY |
| 6 | rust-clippy (ubuntu-22.04, clippy) `cargo clippy -- -D warnings` | lints → none | `rust-checks` entry: `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` | (same entry's uploads) | PARITY |
| 7 | build aarch64-apple-darwin (macos-latest, @stable+target, npm ci) `npx tauri build --target aarch64-apple-darwin`; upload dmg glob (AppImage/deb globs in the same block were no-ops on macOS) if-no-files-found: ignore | dmg → `drift-aarch64-apple-darwin` | macos-bundle (macos-15, rustup 1.98.1 + target): same `npx tauri build --target aarch64-apple-darwin`; upload dmg if-no-files-found: **error** | `macos-bundle-aarch64-apple-darwin` | PARITY-WITH-DELTA: same build command/target; missing dmg now FAILS the job (old could pass silently with zero artifacts — strengthening per Req 7.3); pinned image/toolchain replace macos-latest + @stable |
| 8 | build x86_64-apple-darwin (macos-latest, cross-compiled) — same build command and upload semantics | dmg → `drift-x86_64-apple-darwin` | macos-bundle (macos-15-intel, native x86_64 runner): same target/command; if-no-files-found: error | `macos-bundle-x86_64-apple-darwin` | PARITY-WITH-DELTA: as row 7 + the x86_64 build now runs on a true x86_64 image (old used macos-latest for both targets) |
| 9 | build x86_64-unknown-linux-gnu (ubuntu-22.04, apt deps, npm ci) `npx tauri build --target x86_64-unknown-linux-gnu`; upload AppImage/deb if-no-files-found: ignore | AppImage+deb → `drift-x86_64-unknown-linux-gnu` | dagger-linux-build → `linux-build` entry: npm ci + the SAME `npx tauri build --target x86_64-unknown-linux-gnu` in pinned rust+node containers; module VERIFIES AppImage+deb exist (throw on missing); runner twin re-verifies exported AppImage/deb/linux-build.log (exit 1) | `dagger-linux-build-artifacts` (verified bundles + build log + manifests) | PARITY-WITH-DELTA: same build command (module docstring: "the same command the GitHub workflow runs"); artifact verification added on both sides (old `ignore` could accept an artifact-less green build); bundle extras `file`+`libfuse2` are additions, disclosed in 2.2 notes |
| 10 | workflow scaffolding: push/pull_request on master, concurrency cancel-in-progress, env CARGO_TERM_COLOR/RUST_BACKTRACE | — | all retained verbatim; adds `permissions: contents: read` | — | PARITY-WITH-DELTA (pure hardening; CT14b) |
| 11 | build `needs: [typecheck, rust-check]` (skip-build-if-checks-fail scheduling) | — | no `needs` chains; all six jobs run required and independent | — | PARITY-WITH-DELTA: scheduling/ordering optimization only, not gate strength — every job remains required with no continue-on-error anywhere (CT14v) and a failed gate fails its job (Req 7.3); wasted-build cost is bounded by concurrency cancel-in-progress |
| 12 | host caches: setup-node `cache: npm`, Swatinem/rust-cache@v2 (floating action tags) | — | engine-managed CacheVolumes keyed OS/arch/toolchain/features (npm registry, cargo registry/git/target); nothing imported from/exported to hosts | — | PARITY-WITH-DELTA: intentional ownership change per design ("container caches are engine-managed"); ephemeral CI runners start cache-cold each run — accepted for MVP (design: "Cold-build cost on ephemeral runners is accepted for MVP… the task 2.7 parity gate records wall-time and cache-hit observations without blocking on them") |

**Rows: 12 — PARITY 3 (rows 3, 5, 6), PARITY-WITH-DELTA 9, GAP 0.** Every delta is either a spec mandate (row 4: Req 7.1 names nextest + `cargo test --doc`), a pure strengthening (rows 1, 2, 7, 8, 9, 10: gates that previously could pass without artifacts now fail), or the intended CI-ownership change (rows 11, 12). Environment delta disclosed: Linux checks moved from GitHub ubuntu-22.04 runners to the pinned rust:1.98.1-bookworm container with the byte-identical 7-package apt set; thin jobs now host only the runner's own Docker engine on explicit ubuntu-24.04 (never `-latest`, CT14y).

### Failed checks block artifact acceptance (Req 7.3, 12.8)

- Module side (Linux container): every check step that fails THROWS with embedded evidence — proven live: failing nextest test → `Drift.rustChecks ERROR [32.6s]` with the failing JUnit excerpt (`D:\BuildArtifacts\drift-rdp\dagger\entry-rust-checks-output.txt`); missing bundle after an exit-0 build → `Drift.linuxBuild ERROR [5.7s]` "missing artifacts fail the job (Req 7.2, 7.3)" (`linux-build-run-2-corrupted-glob-output.txt`); cold-launch ok=false exports failure evidence then fails the tier (research.md 2.5 section, CT13n–CT13r + live sabotage 04-failure-path).
- Workflow side: runner twins exit 1 on missing AppImage/deb/linux-build.log (CT14t) and on missing/not-ok/unscreenshot cold-launch status (CT14u + new CT14ab); NSIS and dmg uploads use `if-no-files-found: error` (CT14n, CT14r); no `continue-on-error` anywhere (CT14v); all evidence uploads run `if: always()` (CT14w). Suite evidence: 223 passed / 0 failed today (222 baseline + CT14ab).
- Only the first live Actions run can confirm (not claimed proven here): dagger-for-github@27b130bf launching the pinned engine on a real runner's Docker; windows-2025 `D:\BuildArtifacts` writability + tier-report upload glob; NSIS/dmg upload globs matching the real bundle layout; `jq` availability on the ubuntu-24.04 runner image (assumed preinstalled — the twin depends on it).

### No duplicated full Linux pipeline; native checks kept (Req 7.4, 7.5)

- The new workflow contains no full Linux check/build pipeline outside Dagger: the four dagger-* jobs are thin (checkout → `dagger call … export` → small runner-side verify twin → upload); the only Linux-host shell is the twins' find/jq verification, not a pipeline. The dev-host twin (`Invoke-CI.ps1 -Tier dagger`) is the documented one-command dev path (Req 13.4), not a second CI pipeline.
- Native jobs remain and are new relative to the old workflow's Linux+macOS-only matrix: windows-native (pwsh-only preflight/frontend/rust tiers + NSIS build, Req 7.4/7.5/8.4) and macos-bundle (both architectures). The old workflow had NO Windows job and NO E2E/cold-launch job — windows-native and dagger-linux-e2e are spec additions (Req 8.4, 12.8), not parity obligations. Linux-container results in this record are labeled as such and never substitute for native Windows/macOS validation (native Windows cold launch proven separately in the 2.3 spike; macOS driver proof still blocked, task 2.4).

### Wall-time and cache-hit observations (non-blocking; cold-build cost accepted for MVP)

- Cached replay: linux-build tier total 12.91 s with build layers served from the engine cache (`dagger-tier-report-linux-build.json` totalDurationSeconds 12.91; `entry-linux-build-output.txt` steps 11/12 `CACHED [0.0s]`), bundles + log verified and exported.
- The earlier full-cold linux-build wall time is NOT durably recorded: the 2.2-era per-run report was overwritten by later same-entry runs (per-entry ≠ per-run) and the preserved transcripts (`linux-build-run-1/3`) carry no durations — recorded as not-available rather than invented; same for old-workflow Actions run times (no recorded runs in this workspace). If needed, 11.12 should re-measure once on a cache-cold engine.
- Linux-container cold-launch tier (task 2.5 live run): 258 s total including first-time apt layer + bundle rebuild from changed inputs; the cold-launch harness itself 7.3 s (research.md 2.5 section, 03-live-run).
- Native Windows (task 2.3 spike; native observations, not container): `cargo check` 1m31s, release compile 6m29s, bundles MSI+NSIS produced.
- Cache-hit behavior recorded in 1.3/2.5: identical withExec inputs replay cached stdout (content-addressed engine cache); identical inputs replay the bundle build layers — the 2.5 run never rebuilt what 2.2 cached. Ephemeral GitHub runners start cache-cold each run (no host cache import/export by design); first live Actions run will record the real cold pipeline wall-time (2.5 open item 5).

### Task 2.7 hardening applied (non-blocking 2.6-review suggestion, adopted)

The linux-e2e runner twin previously gated only on `cold-launch-status.json` `.ok == true` (CT14u), while the dev-host tier also fails on ok-without-screenshot (`Invoke-CI.ps1` linux-e2e gate). RED-first: CT14ab added to `scripts/Invoke-CI.Tests.ps1` (suite RED at exactly that assertion: 222 passed / 1 failed), then `.github/workflows/ci.yml` dagger-linux-e2e verdict step extended to also require the exported `cold-launch-screenshot.png` when ok=true; suite GREEN 223/0; `npm run lint` clean. Files: `.github/workflows/ci.yml`, `scripts/Invoke-CI.Tests.ps1`, this section.

## macOS cold-launch + WebdriverIO Tauri-service driver spike (task 2.4, 2026-11-11) — RUN 1 EXECUTED (spike job cancelled at the 60-min timeout; root cause fixed + harness hardened); GATE VERDICT PENDING THE RE-RUN (phase 2 continues)

Outcome: **PARTIAL — no gate verdict yet.** Run 43077191197 (first live Actions run on the fork, phase-1 commits ee3f1af + 80f3b49) cancelled the spike job at the 60-min job timeout, but its exported evidence already establishes at the log level — gate confirmation still requires the re-run — that the built old-stack app cold-launches on the macOS runner (first GPU frame ~1 s after launch) and that the per-run HOME isolation works exactly as designed. The six other jobs went green, closing the task-2.6 obligation (c) confirmations (below). Harness root cause diagnosed from the evidence and fixed; stage caps + bounded reaps added (CT15j–m, RED 232/4 → GREEN 236/0). Requirements 12.1, 12.5, 12.7, 12.8; design.md "Testing Strategy → Delivery gates and brownfield coverage" (E2E driver mandate), "E2E — WebdriverIO + Tauri service (macOS)..." and "CI ownership and reuse decisions". Evidence tree: `D:\BuildArtifacts\drift-rdp\macos-cold-launch\run-43077191197\`.

- **Conversion note (user decision 2026-11-11):** task 2.4 was originally a local 3-hour macOS spike; no macOS host exists in this environment, so it is converted to an Actions macOS-runner job on the fork — build on the runner, prove the WebdriverIO Tauri service embedded server against the built app (not the platform driver), isolate data, export evidence artifacts. The 3-hour investigation cap is kept (runner wait excluded; the job itself is bounded by `timeout-minutes: 60` and a 420 s wdio watchdog).
- **Job design** (`macos-cold-launch-e2e-spike` in `.github/workflows/ci.yml`, appended after `macos-bundle`, native macOS exception — no Dagger): `runs-on: macos-15` (aarch64 — the primary Mac platform and `macos-bundle`'s first target; intel stays bundle-covered), pinned node 22.23.4 / rust 1.98.1 + `aarch64-apple-darwin` target from `.config/ci-versions.json`, `npm ci` at root, then the UNMODIFIED old-stack build with the proven `npx tauri build --target aarch64-apple-darwin` command (no app-code changes — mirrors the 2.3 zero-change result; bundle/dmg gating stays in `macos-bundle`, this job does not re-upload bundles). Then `scripts/macos/cold-launch-spike.sh` runs two stages and ALWAYS exports evidence to `ci-artifacts/macos-e2e-spike/`:
  - *Stage 1 — OS-level built-app cold launch, no WebDriver* (Req 12.8 cold-launch demonstration, twins 2.3 Windows / 2.5 Linux): launch `src-tauri/target/aarch64-apple-darwin/release/rdp-client` under the isolated HOME → poll for a real on-screen window ≥400×300 via `scripts/macos/window-probe.swift` (CGWindowList; bounds+owner-PID are not TCC-gated; tao's tiny helper windows ignored — the 2.3 trap) → `screencapture -x os-cold-launch.png` → SIGTERM, reap, record exit code.
  - *Stage 2 — WebdriverIO embedded-driver session* (the design mandate, Req 12.5/12.8 driver path): `scripts/macos/e2e-spike/` (`wdio.conf.js` + `specs/cold-launch.spec.js`) drives the SAME built binary through `@wdio/tauri-service` with `driverProvider: 'embedded'`, `embeddedPort: 4455`, `startTimeout: 180000`, `statusPollTimeout: 5000` (slow-CI values from the service's own configuration docs), under a 420 s watchdog (macOS runners ship no GNU `timeout`). The spec asserts: session connects and exposes a window handle, `document.title === 'Drift'`, `#root` renders >100 chars of real UI (first usable state), and `saveScreenshot` lands a non-trivial `first-usable-state.png` (>4096 bytes). The harness then verifies the service tore the app down (no lingering `rdp-client`) — the clean-quit leg.
  - *Verdict*: the harness never exits non-zero; it writes `cold-launch-driver-status.json` (`ok`, `reason`, `wdioExitCode`, `osColdLaunch{ok,reason}`, `appQuitClean`) and the WORKFLOW verdict step (if: always()) fails the job on missing status / `ok != true` / ok-without-screenshot (mirrors the CT14u/CT14ab twin pattern; python3 — guaranteed on macOS runners). Evidence artifact `macos-cold-launch-e2e-spike-evidence` uploads `if: always()` with `if-no-files-found: warn` (the verdict step carries the gate). Failure screenshot via `screencapture -x driver-failure-screen.png` on any non-pass.
- **Isolation mechanism (Req 12.7):** the harness scopes `HOME` into `ci-artifacts/macos-e2e-spike/run-home` and `TMPDIR` into the evidence tree. On macOS the Tauri/dirs stack resolves app data under `$HOME/Library/Application Support/com.drift.app` and WKWebView keeps its state under `$HOME/Library/WebKit/<identifier>` — the per-run HOME isolates both. (The 2.3 caveat — Windows Known Folder APIs ignoring env overrides — is Windows-specific; Linux XDG overrides were proven respected in 2.5; macOS HOME overrides are the documented dirs-crate behavior for `data_dir`.) Cleanup touches only the run's own resources: the built `rdp-client` process and the evidence tree, on an ephemeral runner.
- **Driver choice rationale (12.8 "documented supported driver path"):** design.md mandates the WebdriverIO Tauri service embedded macOS server ("direct tauri-driver does not drive WKWebView on macOS"). Vendor docs verified 2026-11-11 (unpkg `@wdio/tauri-service@1.5.1` docs: platform-support.md, configuration.md): on macOS the service offers exactly two providers — `embedded` (default) and a third-party keyed driver requiring `CN_API_KEY` (a secret; CT14x forbids secrets in CI); `external` (cargo tauri-driver) is Windows/Linux-only. Hence `driverProvider: 'embedded'` is the only secrets-free macOS path.
- **⚠ Feasibility discovery (the spike's core open risk, known BEFORE the live run):** the same vendor docs state the embedded server runs *inside the app* via the `tauri-plugin-wdio-webdriver` Rust plugin — "The embedded provider requires tauri-plugin-wdio-webdriver in your app — if the plugin is missing, the embedded server never comes up and the session times out" (configuration.md; the macOS section registers it `#[cfg(debug_assertions)]` with a `wdio-webdriver:default` capability). The OLD-STACK app does not embed that plugin, and adding it is OUT of task 2.4's authorized boundary: design.md "Allowed Dependencies" does not list it as a Rust dependency ("No other general-purpose Rust crate is added by default"), and Req 12.10 keeps production builds free of test control. So the live run is EXPECTED to show one of exactly two honest outcomes:
  1. `ok=false, reason=os-cold-launch-failed:*` — the built app itself fails to cold-launch on the runner (would block the upgrade outright), or
  2. OS cold launch PASSES (stage-1 evidence: window probe, screenshot, graceful quit) and the driver session FAILS with the embedded-server timeout signature (`wdio-exit-*` after start-timeout, or watchdog kill) — recorded macOS cold-launch proof plus documented-blocking-driver evidence: the supported macOS driver path requires a design amendment authorizing the debug-gated `tauri-plugin-wdio-webdriver` as an E2E-only app dependency (a group-11/12 decision), or an alternative driver. Either way the evidence decides; the job never fakes a pass. (A surprise `driver-session-passed` — plugin-free embedded session — would falsify the vendor doc claim and is equally welcome.)
- **Pins:** no new Actions — the job reuses the three SHA-pinned actions already verified in 2.6 (`actions/checkout@3d3c42e…` v7.0.1, `actions/setup-node@949feb2…` v7.1.0, `actions/upload-artifact@cf430e0…` v7.0.2). npm pins live in `scripts/macos/e2e-spike/package.json` as EXACT versions (CT15e forbids ranges): `@wdio/tauri-service 1.5.1` (latest, WebdriverIO-org trusted publisher), `webdriverio 9.31.9` + `@wdio/cli/local-runner/mocha-framework 9.31.9` + `@wdio/spec-reporter 9.31.2` — the service's own dependency baseline (its manifest pins `webdriverio 9.31.9` exactly and `@wdio/spec-reporter 9.31.2`; no 9.31.9 of spec-reporter exists; verified against the npm registry 2026-11-11). Runner `macos-15` explicit, node/rust from ci-versions.json, no secrets, no continue-on-error.
- **Why a scoped `scripts/macos/e2e-spike/` package instead of root devDependencies:** (a) the dev host cannot reach the npm registry from the task harness, so the root `package-lock.json` cannot be regenerated — root `npm ci` (dagger-frontend, windows-native, macos-bundle) must stay untouched; the job runs `npm install` against the exact direct pins (transitives float within their declared ranges — disclosed; a proper root lockfile lands with the group-11/12 suite); (b) the old-stack dependency graph stays unmodified for the spike; (c) `scripts/macos/` twins `scripts/linux/` and avoids creating a top-level `e2e/` directory — verified no `e2e/`-existence logic remains in `.dagger/src/index.ts` (the 2.5 rework replaced the old throw-guard), and CT15h now guards the absence explicitly.
- **Tests (RED-first):** CT15a–CT15i added to `scripts/Invoke-CI.Tests.ps1`; CT14w upload-count 7→8 and CT14aa reworded from "BLOCKED" to the converted-spike wording. RED captured against the pre-edit workflow: **222 passed / 10 failed** (CT14w, CT14aa, CT15a, b, d, e, f, g, h, i — transcript `D:\BuildArtifacts\Temp\opencode\ct15-red-transcript.txt`); GREEN after wiring: **232 passed / 0 failed**; `npm run lint` clean.
- **What the live run must show (re-run checklist):** job `macos-cold-launch-e2e-spike` completes on `norandom/1-1` WITHIN the job timeout (harness self-terminates: stage 1 ≤ 300 s, stage 2 ≤ 600 s); artifact `macos-cold-launch-e2e-spike-evidence` downloadable with `cold-launch-driver-status.json`, `harness-timeline.txt` (new), `swiftc-compile.log` (new), `window-probe.bin`, `os-window-probe.txt`, `os-cold-launch.png`, `wdio-output.log`, `app-binary-sha256.txt`, `runner-os.txt`, and (on failure) `driver-failure-screen.png`; then record the verdict per the feasibility matrix above.

### Run 1 (43077191197) — results, diagnosis and hardening (2026-11-11)

- **First live validation of the whole 2.6 thin workflow — six jobs GREEN:** dagger-frontend ✓, dagger-rust-checks ✓ (7m49s), dagger-linux-build ✓, dagger-linux-e2e ✓ (cold-launch gate incl. the runner-side jq verdict), windows-native ✓ (pwsh tiers + NSIS installer published), macos-bundle ✓ BOTH archs. This closes task 2.6's open obligation (c) from the Implementation Notes: dagger-for-github@27b130bf launching the pinned engine on a real runner's Docker ✓, windows-2025 `D:\BuildArtifacts` writability + tier-report/NSIS upload globs ✓, jq availability on ubuntu-24.04 ✓. Run URL: https://github.com/norandom/1-1/actions/runs/43077191197
- **Fork trigger finding:** PUSH events to the fork never started the workflow (Actions + workflow enabled; push to master with `on: push` present — no run). Only `workflow_dispatch` works; the parent added the trigger (commit 80f3b49) and dispatched manually. Any future re-runs on the fork must be dispatched, not pushed. (Upstream/CREVIOS push behavior untested — fork-only observation.)
- **Spike job cancelled at the 60-min job timeout** (ran 1h0m21s; annotations: "exceeded the maximum execution time of 1h0m0s", "operation was canceled"). All steps green through the old-stack `npx tauri build` + spike `npm install` (~≤10 min in); the spike step and verdict step were cancelled mid-flight; the evidence upload still ran (`if: always()` held).
- **Positive evidence banked from the cancelled run** (log-level; gate confirmation pending the re-run):
  - *Built-app cold launch on macOS 15.8.2 arm64 (runner `sw_vers`/`uname`: Darwin 24.6.0 RELEASE_ARM64_VMAPPLE):* `os-launch-app.log` (mirrored in `run-home/Library/Logs/com.drift.app/Drift.log`) — 18:33:12 `App data directory: …/ci-artifacts/macos-e2e-spike/run-home/Library/Application Support/com.drift.app`; 18:33:13 `GPU renderer created: adapter=Apple Paravirtual device, format=Bgra8UnormSrgb, present_mode=Fifo, size=1024x681`; `[GPU] First frame rendered successfully!` — one second after launch, zero code changes (the 2.3 zero-change result now also observed on macOS).
  - *Isolation (Req 12.7) works on macOS:* the app resolved its data dir INSIDE the per-run HOME, and the app's own Drift.log landed under `run-home/Library/Logs/com.drift.app/` in the exported artifact — the HOME-override mechanism behaved exactly as designed.
  - *screencapture works on the runner (no TCC-black frames):* `os-cold-launch-failure.png` 138,235 bytes, 1024×768, 460 distinct sampled colors — real rendered content on the runner display (window-level confirmation still needs the fixed probe).
- **Diagnosis — where the 45+ minutes went** (two harness bugs, both mine, both fixed):
  1. *REPO_ROOT mis-resolution:* `os-window-probe.txt` contains 60 identical lines `error: error opening input file '/scripts/macos/window-probe.swift' (No such file or directory)` — the workflow step runs a CR-stripped copy at `/tmp/cold-launch-spike.sh`, so `dirname "$0")/../..` resolved to `/`, and every probe invocation failed instantly (the probe path was `/scripts/macos/window-probe.swift`). The reviewer's swift-compile-cost hypothesis pointed at the right loop but the wrong mechanism: each iteration failed fast; compile seconds did not burn the hour.
  2. *Unbounded wait on a healthy GUI app:* with `WINDOW_FOUND=false` after 60 iterations (~60–120 s), the failure branch ran `wait "$APP_PID"` on the still-running app (which had rendered its first frame at 18:33:13 and would run forever) — that `wait` blocked ~45+ min until GitHub cancelled the job. No status JSON and no `wdio-output.log` exist in the artifact, confirming the harness died inside stage 1 (stage 2 never started).
- **Hardening applied (verdict semantics, isolation, driver choice and the two-stage design UNCHANGED; RED 232/4 → GREEN 236/0, CT15j–m):**
  1. `DRIFT_REPO_ROOT: ${{ github.workspace }}` now passed by the workflow and preferred by the harness (script-relative resolution kept only as a fallback for direct repo invocations; an unresolvable root writes `harness-repo-root-unresolved` and exits cleanly).
  2. The window probe is compiled ONCE (`swiftc … -o "$EVID/window-probe.bin"`, compile log exported; `swiftc-compile-failed` is an explicit failure) and the poll loop runs the binary — milliseconds per poll, no interpreter per iteration.
  3. Hard stage wall-time caps write explicit failure statuses and exit cleanly so the verdict step fails the job with evidence — the GitHub job timeout is never the terminator: `STAGE1_CAP_SECS=300` (reason `stage1-timeout-no-window`), `STAGE2_CAP_SECS=600` (reason `stage2-timeout`; the 420 s wdio watchdog remains the inner bound for the session itself).
  4. `kill_bounded` (TERM → ≤15 s → KILL-9 → reap, KILLED flag + reaped exit code recorded) is now the ONLY teardown path — the two remaining `wait "` calls live inside `kill_bounded` and the wdio watchdog; a bare wait on a live GUI process can never hang the job again.
  5. `harness-timeline.txt`: UTC-stamped stage transitions (probe compile, app launch, poll result, stage verdicts) — run 1's artifact had no timeline, which lengthened this diagnosis.
  6. `timeout-minutes: 60` KEPT (not bumped): post-fix worst case ≈ build ~10 min (run 1: build + deps all done by 18:33) + stage 1 ≤5 + stage 2 ≤10 + verdict/upload ≈ 30 min; with the harness self-terminating, the smaller timeout preserves a faster failure signal (3-h-cap spirit) — 90 min would only add dead hang room that stage caps already made unreachable.

### Phase-2 evidence — re-run (fill from the re-dispatched run; nothing below is claimed yet)

- Re-run URL: `PENDING (phase 2 / phase 3 resume)`
- Job result + stage timeline from `harness-timeline.txt`: `PENDING`
- Artifact inventory (actual files + sizes): `PENDING`
- OS-level cold launch gate (probe output, screenshot, quit path/exit code, `osColdLaunch.ok`): `PENDING`
- Embedded-driver session (wdio output excerpt, timeout signature or pass, service logs): `PENDING`
- Verdict (12.8): `PENDING` — macOS built-app cold launch: observed in run-1 logs, gate-unconfirmed; supported driver path: unproven-until-run
- Iterations (job-bug fixes → re-push), if any: run 1 → CT15j–m hardening (this section); further: `PENDING`

