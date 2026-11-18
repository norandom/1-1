# Readable remote desktop roadmap

Updated 2026-11-18. The maintainer approved the folder-sharing and session-actions task plans for implementation. Pause and Resume are the first Windows test milestone. The workspace delta-sync design is deferred until the ordinary share measurements are available. The [Windows preview handover](../handover.md) lists the existing release work and outstanding real-host checks.

## Specs (dependency order)

- [ ] **RDP folder sharing.** Start with the shared server identity check. Then expose one selected folder, read-only by default, with explicit read/write access available. Verify ordinary copies in both directions on the Windows and two Linux hosts. Measure bytes, concurrent requests and typing latency before deciding on a segmented copy command. [Requirements](../specs/rdp-folder-sharing/requirements.md) · [Design](../specs/rdp-folder-sharing/design.md) · [Tasks](../specs/rdp-folder-sharing/tasks.md)
- [ ] **Session actions.** The first Windows milestone is Pause and Resume in the existing status bar. This can proceed alongside the server identity work. The saved-host credential test reuses that identity check and can ship before the file share. [Requirements](../specs/rdp-session-actions/requirements.md) · [Design](../specs/rdp-session-actions/design.md) · [Tasks](../specs/rdp-session-actions/tasks.md)
- [ ] **Workspace delta sync.** If the measured mount baseline justifies it, compare native rsync and Unison over opt-in SSH for explicit uploads and downloads. Verify shifted-block savings and preservation of concurrent edits before choosing a tool or funding a custom peer. [Requirements](../specs/workspace-delta-sync/requirements.md) · [Design](../specs/workspace-delta-sync/design.md)

The new proposals carry their own approval state. The prior spec's 62 completed numbered tasks out of 77 and its separate bandwidth experiment retain their original scope. Immediate acceptance work on the Windows client still includes Windows and FreeRDP shrink/restore, real-host clipboard Resync after reconnect, and the active and unattended bandwidth comparisons.

The shared server identity check and saved-host fingerprint review are implemented. On 18 November the Windows client connected, rendered and reconnected on all three staged hosts. The IronRDP host confirmed both resize steps. Windows and FreeRDP still need native shrink and restore confirmation. [Three-host results](../specs/rdp-folder-sharing/validation.md)

## November 17 implementation and release scope

User clarification on 2026-11-17: no font defect is currently reported; the
updated software awaits testing. The next priority is real connection and smart
resize on all three staged hosts. Distinguish remote resolution changes from
local fit scaling, and inspect text rendering after resize before diagnosing
font smoothing. The full fixed-text matrix remains later acceptance work.

## Outcome

The product is now **1-1**, pronounced **zero** (user-confirmed 2026-11-17).
Current user priority after the native resize observation: maximize vertical
workspace, move display/resize controls to a compact menu, persist professional
per-host experience/network options, and size the initial dynamic connection to
the real viewport. Requirements 14.7-14.10 and additional tasks 11.0d-f capture
these directed changes; they do not erase pending real-server/release acceptance.

Readable remote text, predictable window and session navigation, and reliable
operation against the existing Windows 11, FreeRDP-based Linux, and IronRDP-based
Linux targets. ClearCodec support, font smoothing, glyph caching, and display
scaling are separate concerns. A requested flag is not proof of server acceptance.

## Delivery order

| Stage | Existing scope | Completion evidence | State |
|---|---|---|---|
| 1. Session navigation repair | Remediate 8.6; requirements 11.2, 11.4, 11.6, 11.7, 11.8, 11.9 | Two sessions cannot exchange frames; dashboard/return restores the correct display without reconnecting; close releases resources; terminal restart retains identity | Reviewed and verified |
| 2. Bounded delivery | 9.5 | One in-flight frame, accumulated damage, generation-aware acknowledgement, visible five-second stall and full-refresh recovery; delayed/dropped/stale acknowledgement tests | Reviewed and verified |
| 3. Font smoothing | 10.1 then 10.2 | Default on; saved toggle; `.rdp` import; legacy default and explicit off preserved; outgoing flag verified; help describes server override and reconnect | Implementation reviewed; native save/reopen verified; server rendering acceptance pending |
| 4. Pixel fidelity | 10.3 | Native pixels and aspect-preserving fit; correct mouse coordinates and scrolling at 100/125/150/200% DPI; no double scaling | Implementation reviewed; automated checks pass; physical validation pending |
| 5. Three-target readability | Relevant 11.x interoperability/text/performance evidence | Same fixed text page, font, remote resolution and server scaling; record actual codec and smoothing request; compare decoded pixels, canvas and screenshots, plus human readability feedback | Pending |
| 6. Release completion | Remaining 11.x journeys and native/container gates | Required CRUD, keyboard, recovery, production-build and platform evidence; unresolved findings explicitly closed or blocked | Pending |

The existing plan includes additional Windows Server and grd coverage. Keep those
release obligations; do not silently treat the three staged targets as equivalents.
No new glyph rendering subsystem, codec implementation, dependency upgrade, or
test framework is planned for text readability. Use upstream capabilities.

Latest smart-resize evidence: independently reviewed implementation committed
`c7ef77f`; native Windows 1920x1080 to 3180x1271 confirmed with desktop redraw.
User Escape interrupted restore. Follow-up headless three-host run at
`group11/resize-smoke/smoke-20261117-135059` passed connection gates and redaction,
confirmed IronRDP 3064x1264 to 1280x720 and back, but Windows/FreeRDP resize
confirmation timed out after requests were sent. Do not call the overall smoke
PASS a three-host resize PASS. Read-only investigation did not isolate the
harness/native discrepancy; recording ignored smoke write errors/termination is
the next diagnostic step. Test app is closed, isolated credentials removed,
Computer Use reset.

Professional-control delivery: branding is committed in `0e128a1`, and task 11.0d
profile persistence/protocol flags in `35a30cd`, independently reviewed with
304 default / 322 test-support Rust tests and 490 frontend tests passing.
11.0e compact menu/form is committed in `335f9a8`; 11.0f initial viewport sizing
is committed in `4fef31e`. The native follow-up confirmed unclipped menus,
per-host WAN persistence, and Windows initial request/actual size of 940x736.
Fixed mode retained that resolution while maximizing. Escape also opened the
disconnect dialog when dismissing a menu; this integration finding is being
repaired before continuing the remaining native resize checks. The test app is
closed, temporary credentials removed, and Computer Use reset.
These checks do not establish full three-host resize or text-quality acceptance.

## Review findings and scope control

- The prior spacing, dashboard refresh and native window-control cleanup remains
  uncommitted in three files. Save/tree refresh and keyboard tab close are included
  in reviewed task commits. Preserve the remaining UI changes.
- New Library UI remediation is reviewed and natively verified, including
  creation, restart persistence and selected-row readability. This removes the
  product prerequisite for the CRUD journey; the shared harness is still pending.
- Start with a small working user journey, then add the next capability. Tests
  must exercise production behavior rather than merely validate fixture constants.
- Keep task-local review and focused validation. Run platform/group gates at
  their existing boundaries, not after every cosmetic change. Record real failures
  and avoid rerunning unaffected checks without a reason.
- Existing group-boundary holds remain in effect. Group 10 is complete at the
  implementation boundary; the user explicitly resumed group 11 on 2026-11-17.
- Group 11 preflight found that its assumed shared UI harness does not exist.
  Repair New Library in the owning 8.3 boundary first, then deliver the already
  approved harness as separate 11.0a Windows/shared, 11.0b macOS and 11.0c Linux/
  matrix slices. Only then run 11.1's storage journeys. This separates omitted
  prerequisites from acceptance work; it introduces no new product/driver choice.
- Keep the delivery order above: after the native UI repairs, resolve B3-7 and
  resume the staged real-RDP/readability checks. The 11.0 harness slices are
  prerequisites for later release journeys, not for inspecting the staged servers.
- Computer Use runs only for a bounded native check. Close the isolated test app,
  send the completion summary, and reset the session afterward; restart only when
  another UI check is needed (user preference, 2026-11-17).
- Checkboxes in tasks.md remain authoritative for original implementation tasks.
  This roadmap does not mark unfinished tasks complete or reopen unrelated work.

## Current evidence

After task 10.3, 49 of 61 original subtasks are checked; 11.1-11.12 remain open.
Earlier three-target smoke runs established connectivity/frame/reconnect behavior,
not completed text-quality acceptance. The connector now requests the saved font
smoothing preference; help explicitly describes server override and next connection.

Group 10 passed independent task reviews. Final evidence: 463 frontend tests,
289 default Rust tests and 307 test-support Rust tests (four existing skips each),
lint/typecheck/build/fmt/doctests; Clippy retains two existing warnings. Native
test binary built successfully. Pixel fixtures cover full/dirty RGBA and pointer
coordinates at four DPIs in both modes; wheel regression verifies cancellation,
single forwarding and cleanup. Review caught and corrected passive React wheel
handling that would otherwise pan both local and remote desktops.

The isolated native UI launch was rejected by automatic approval policy with
"blocked by policy" and no further reason. No test app or Computer Use session
started. Preference save/reopen and display controls still need native UI smoke;
actual canvas readback and physical/server readability remain acceptance work.
Do not treat unit fixtures as real-server or physical-display quality evidence.
Artifacts: `D:/BuildArtifacts/drift-rdp/group10`.

Group 11 follow-up: after the user's explicit resume, isolated native launch
succeeded. New Library remediation passed independent review and 471 frontend
tests plus lint/typecheck/build. Native creation, keyboard submit, rail cancellation,
restart persistence and selected-name contrast passed. Font smoothing off saved
and reopened correctly. The display-mode check found faint light-theme labels
and a shortcut collision with the performance HUD; fix these before harness work.
Both test-app sessions were closed and Computer Use reset. Real-server text
quality and canvas-readback acceptance remain unverified. Follow-up artifacts:
`D:/BuildArtifacts/drift-rdp/group11`.

The display-control remediation is independently reviewed: 475 frontend tests and
lint/typecheck/build pass. Native inspection confirmed readable labels, toolbar
separation and Native pixels selection with the synthetic frame intact. Keyboard
focus via the display shortcut remains inconclusive through Computer Use; a
separate Ctrl+Shift+P probe opened WebView2 Print and was cancelled. Keep native
shortcut acceptance open in 11.2. The test app and Computer Use were closed/reset.

Task 9.5 passed independent review after correcting reactivation damage and
stall-state handling. Checks: 443 frontend tests, 286 default Rust tests,
304 test-support Rust tests (four existing skips each), lint/typecheck/build,
formatting and doctests; Clippy retains two existing warnings. Actor tests
reproduce delayed/dropped acknowledgements and smaller/same-size reactivation,
then verify exact latest-image pixels. Final native test-support smoke confirmed
dashboard return, two-session rendering and adjacent close without extra connects.
Artifacts: `D:/BuildArtifacts/drift-rdp/task95`. The test app and Computer Use
session were closed after verification.

Session remediation passed independent review, 438 frontend tests, lint,
typecheck and production frontend build. Native test-support runs verified
dashboard return with the same session ID and rendered image, direct A-to-B
attachment, and closing B back to A. Runtime tests cover frame isolation,
failure/retry, restart, and disposal races. Test artifacts live under
`D:/BuildArtifacts/drift-rdp/session-navigation`.

B3-7 resolved with independent Rust and frontend review. Real failures retain
Error without synthetic frames; Rust error objects normalize at UI ingress and
the footer follows actual lifecycle state. Native invalid-host check verified
Connection Error with the DNS cause, Error in tree/tab/footer and zero active
sessions. Explicit test mocks still render. Tests: 291 default Rust, 309
test-support Rust (four existing skips each), 487 frontend, lint/typecheck/build,
fmt/doctests/Clippy (two existing warnings). Evidence: group11/failure-* and
review-failure-* under D:/BuildArtifacts/drift-rdp. Test app and Computer Use closed.
Three-host connector smoke passed; real app smart-resize is next. Its headless
smoke did not negotiate display-control DVC and cannot prove resize support.

UI cleanup checks passed on 2026-11-17: 428 frontend tests, lint, app typecheck,
frontend build, Rust formatting and Clippy (two existing warnings). Native debug
preview confirmed spacing and a single window-control row; live RDP and populated
dashboard interaction were not tested in that review.

## Delivery queue recorded on 2026-11-17

Completed and independently reviewed: professional graphics depth/compression (11.0h), one-second status-bar hover delay (11.0i), directional clipboard Resync/recovery (11.0j), and native text plus images/screenshots (11.0k). These are implementation results; real-host clipboard acceptance is still open.

The diagnostic repair subsequently landed. All three targets connected and the IronRDP host confirmed shrink and restore. Native Windows and FreeRDP round trips and real-host clipboard checks are still pending. See the [handover](../handover.md) for the test sequence.

Remaining release work: 11.0a-c shared/native test harness adapters, 11.1-11.3 storage/keyboard/session journeys, 11.4-11.6 server interoperability, 11.7-11.10 DPI/text fidelity, 11.11 performance and 11.12 final platform/evidence gates. The three staged hosts do not replace the Windows Server/grd requirements. Keep tasks unchecked until their full evidence exists.

Later feature candidates: remote DPI and keyboard layout controls, and clearer negotiated-session diagnostics. File transfer, multi-monitor, audio and drive redirection require separate integration scope. Prioritize validating the delivered features before adding these.

User removed the build guard: no mandatory storage checks or budget stops. Occasional explicit package-only cleanup while idle is sufficient; preserve evidence and active output.
