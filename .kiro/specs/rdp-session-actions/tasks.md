# Session actions implementation plan

The first testable change is **Pause display** in the existing status bar. Its four tasks can run before the credential check or folder sharing. The credential check consumes the same-socket server verifier, private endpoint pins and **Review server identity** action from [folder-sharing task 1](../rdp-folder-sharing/tasks.md). It requires no folder grant and can ship as soon as that shared task is available.

Each numbered subtask is one implementation or validation unit, sized for roughly one to three hours. Run the focused regression with the change that introduces it. Coordinate edits to shared command registration, frontend types, the saved-host menu and the status bar with the folder-sharing work.

The independent Kiro task-plan review passed on 18 November 2026. The review covered all 19 acceptance criteria, task dependencies and Windows test milestones.

- [ ] 1. Pause and resume one connected display
- [x] 1.1 Add manual minute previews to the existing idle policy
  - On an explicit pause suppress immediately, even with automatic preview off. Adopt an already suppressed interval, close an open automatic preview and keep manual intent on a quiet or costly screen.
  - Use virtual time to show two-second update windows about 60 seconds apart. Coalesce late ticks and return to the saved automatic policy on deliberate resume or a fresh connection.
  - Exercise the capability flag and ignored-suppression rule. After the two-second grace period, desktop paints in three consecutive ten-second suppressed intervals trigger a return to live. Cursor changes, preview windows and resize redraws cannot trigger it.
  - _Boundary: Idle preview policy_
  - _Requirements: 2.1, 2.2, 2.7, 2.8_
- [x] 1.2 Apply pause and input to the same session writer
  - Route idempotent Pause and Resume commands through the connected actor. Reject a stale session ID. Keep the framebuffer, socket reads, graphics acknowledgements, clipboard transfers and Resync running.
  - Classify deliberate remote keystrokes, clicks, scroll and paste separately from pointer movement, focus, local modifier release and background clipboard updates. When suppressed, send Allow before the triggering input. When a preview is already open, keep output live without sending a duplicate Allow. Forward the triggering input exactly once in either case.
  - Capture two sessions' wire events with pending clipboard traffic. Assert ordered wake/input, continued acknowledgements, unchanged manual state on incidental activity and a cleared manual override after reconnect. The new session starts from its saved automatic preference.
  - _Boundary: Session actor and command integration_
  - _Requirements: 2.1, 2.2, 2.3, 2.5, 2.6, 2.8, 2.9_
- [x] 1.3 Keep manual pause across resize and expose the actual image age
  - Track negotiated support at activation and reactivation. Publish actor-confirmed manual state, capability, reason and the time of the last completed desktop paint even when automatic preview is off.
  - Coalesce resize requests, permit a bounded redraw at the new desktop bounds, and resume suppression after a completed new-size paint or the two-second window. Clear transient state on disconnect and restore live output if support disappears.
  - Verify resize during suppression and during a preview. The old image and its age survive until a real desktop paint. A cursor, clipboard event or elapsed timer cannot advance the timestamp.
  - _Boundary: Session actor graphics and status projection_
  - _Requirements: 2.2, 2.4, 2.6, 2.7, 2.8_
- [ ] 1.4 Integrate the control with the existing status bar and keyboard handling
  - Bind Pause and **Resume live** to the selected tab. Show update age or “Last update unknown” and an accessible reason when unavailable. Enable it from the actor's negotiated capability independently of automatic preview.
  - Route view return, locally consumed menu keys and release of earlier forwarded modifiers separately from deliberate remote input. Preserve remote key state, the existing auto-hide delay and the viewport used for smart resize.
  - Frontend interaction tests switch tabs, use the control by keyboard, show and hide the bar and return focus. The selected session receives the expected command once, the other stays unchanged, and the bar occupies its measured height.
  - _Boundary: Session UI and IPC integration_
  - _Requirements: 2.1, 2.4, 2.5, 2.6, 2.7, 2.9_

- [ ] 2. Test the credential saved for one host
- [ ] 2.1 (P) Stop one authenticated probe before desktop establishment
  - Open a single connection to the saved endpoint and consume the verified identity on that same TLS socket. Drive the pinned NLA sequence with the saved identity only. Return typed evidence from received server messages and close the socket before reporting.
  - Use a counting protocol fixture. A changed or unknown identity shows no credential transmission. Completion and cancellation show no desktop negotiation, reconnect, alternate credential, channel registration or change to an existing desktop.
  - This driver can run alongside the pause work once its external verifier is ready. Folder-sharing task 1 owns identity review and pin persistence.
  - _Boundary: Credential probe driver_
  - _Depends: rdp-folder-sharing task 1_
  - _Requirements: 1.1, 1.5, 1.7, 1.9_
- [ ] 2.2 Classify the server's credential evidence and redact diagnostics
  - Classify a valid HYBRID_EX early success as accepted. Only a decoded server TSRequest with an allowlisted credential-specific failure counts as rejected. Generic AccessDenied, plain HYBRID client completion, interactive-only sign-in, malformed replies and transport loss remain inconclusive.
  - Allowlist returned reasons and suppress sensitive protocol traces in verbose mode. Use fragmented server messages and a sentinel credential to verify the outcome and prove UI payloads, diagnostics and exported connection data omit secrets and token dumps.
  - _Boundary: Credential probe driver_
  - _Requirements: 1.3, 1.4, 1.5, 1.9_
- [ ] 2.3 Integrate the probe with saved-host edits, keyring and deadline
  - Accept only a saved host ID. Start one 30-second deadline before store or keyring lookup, coalesce repeated requests per host revision and release locks before network I/O.
  - Invalidate on endpoint, user, domain, saved secret, deletion and replacement/import changes. Cancel the old exchange, ignore late blocking lookups and reload effective values after a partial save. Only publish the result of the still-current revision.
  - Race duplicate requests and profile edits with delayed keyring and protocol fixtures. Observe one attempt for the unchanged revision, bounded completion on timeout, closure of an open socket and zero late writes or stale results.
  - _Boundary: Saved-host commands, credential store and probe integration_
  - _Requirements: 1.1, 1.2, 1.5, 1.6, 1.7, 1.9, 1.10_
- [ ] 2.4 Show the result in the selected saved host's menu
  - Add **Test credentials** with Checking, green Accepted, red Rejected and gray Unable to verify with its reason. Include the host name and announce the result as text. Offer the shared identity review action on the applicable gray result and let the operator explicitly retest.
  - Test keyboard activation, repetition while pending, switching hosts, changing a saved profile and deletion/reload while the request is outstanding. Results appear only on the matching revision. Opening the menu keeps the existing desktop's remote keys balanced.
  - _Boundary: Saved-host menu and IPC integration_
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.8, 1.10, 2.9_

- [ ] 3. Verify the actions in the Windows build
- [ ] 3.1 Exercise Pause on an active remote desktop
  - Run focused frontend tests, native Windows Rust nextest and doctests for the changed paths. From the built Windows client, exercise two preview cycles, local menu use, native resize, clipboard Resync, first-key wake and reconnect on an authorized host.
  - Record the client/server builds, negotiated capability, visible state, preview times, actual resize and observed input. Record a host lacking support as unavailable and use a supporting host to verify pause behavior.
  - This check can run as soon as the pause control is integrated. Schedule the desktop and shared Windows build separately from other tasks using the same resources.
  - _Boundary: Windows desktop acceptance_
  - _Depends: 1.4_
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9_
- [ ] 3.2 Verify one manual credential test per invocation
  - Run focused frontend tests, native Windows Rust nextest and doctests for the credential changes. On an authorized Windows-client test against a known host and controlled test identity, record protocol, elapsed time, visible result and continuity of any open desktop.
  - Exercise a known accepted identity where the server supplies positive evidence. Use a controlled denial case and an unreachable endpoint. Record the evidence a live host actually returns, including inconclusive results. Use protocol fixtures to verify conclusive outcomes the host cannot demonstrate.
  - _Boundary: Windows credential acceptance_
  - _Depends: 2.4_
  - _Requirements: 1.1, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9_
