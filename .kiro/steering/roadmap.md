# Readable remote desktop roadmap

Updated 2026-11-17. The user requested a focused roadmap and authorized proceeding.
Continue the approved `windows-clearcodec-royalts-e2e` spec; no new spec or
protocol rewrite is needed. Existing completed work remains evidence, not a
substitute for integrated user journeys.

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
| 3. Font smoothing | 10.1 then 10.2 | Default on; saved toggle; `.rdp` import; legacy default and explicit off preserved; outgoing flag verified; help describes server override and reconnect | Pending |
| 4. Pixel fidelity | 10.3 | Native pixels and aspect-preserving fit; correct mouse coordinates and scrolling at 100/125/150/200% DPI; no double scaling | Pending |
| 5. Three-target readability | Relevant 11.x interoperability/text/performance evidence | Same fixed text page, font, remote resolution and server scaling; record actual codec and smoothing request; compare decoded pixels, canvas and screenshots, plus human readability feedback | Pending |
| 6. Release completion | Remaining 11.x journeys and native/container gates | Required CRUD, keyboard, recovery, production-build and platform evidence; unresolved findings explicitly closed or blocked | Pending |

The existing plan includes additional Windows Server and grd coverage. Keep those
release obligations; do not silently treat the three staged targets as equivalents.
No new glyph rendering subsystem, codec implementation, dependency upgrade, or
test framework is planned for text readability. Use upstream capabilities.

## Review findings and scope control

- The prior five-file UI cleanup is uncommitted: restored Tailwind spacing,
  save/tree/dashboard refresh, keyboard tab close, and one native window-control
  row on Windows/Linux. Preserve it during the session repair.
- Missing New Library UI remains a separate 8.x integration remediation before
  the CRUD acceptance journey; it does not block the text-rendering changes.
- Start with a small working user journey, then add the next capability. Tests
  must exercise production behavior rather than merely validate fixture constants.
- Keep task-local review and focused validation. Run platform/group gates at
  their existing boundaries, not after every cosmetic change. Record real failures
  and avoid rerunning unaffected checks without a reason.
- Existing group-boundary holds remain in effect. Task 9.5 completes the group-9
  checkpoint; group 10 follows explicit resume, starting with 10.1.
- Computer Use runs only for a bounded native check. Close the isolated test app,
  send the completion summary, and reset the session afterward; restart only when
  another UI check is needed (user preference, 2026-11-17).
- Checkboxes in tasks.md remain authoritative for original implementation tasks.
  This roadmap does not mark unfinished tasks complete or reopen unrelated work.

## Current evidence

After task 9.5, 46 of 61 original subtasks are checked;
10.1-10.3 and 11.1-11.12 remain open. Earlier three-target smoke runs established
connectivity/frame/reconnect behavior, not completed text-quality acceptance.
The current connector does not yet request font smoothing.

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

Release blocker rediscovered: a real connection to an invalid host falls back
to synthetic frames and reports Connected (existing B3-7 behavior in session.rs).
Resolve this before real-server acceptance. Native failure-state validation was
therefore inconclusive; do not present the synthetic image as RDP evidence.

UI cleanup checks passed on 2026-11-17: 428 frontend tests, lint, app typecheck,
frontend build, Rust formatting and Clippy (two existing warnings). Native debug
preview confirmed spacing and a single window-control row; live RDP and populated
dashboard interaction were not tested in that review.
