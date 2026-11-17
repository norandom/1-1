# Readable remote desktop roadmap

Updated 2026-11-17. The user requested a focused roadmap and authorized proceeding.
Continue the approved `windows-clearcodec-royalts-e2e` spec; no new spec or
protocol rewrite is needed. Existing completed work remains evidence, not a
substitute for integrated user journeys.

User clarification on 2026-11-17: no font defect is currently reported; the
updated software awaits testing. The next priority is real connection and smart
resize on all three staged hosts. Distinguish remote resolution changes from
local fit scaling, and inspect text rendering after resize before diagnosing
font smoothing. The full fixed-text matrix remains later acceptance work.

## Outcome

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
