# RDP session action requirements

## Introduction

A professional using 1-1 can test a saved credential before opening a remote desktop and can switch a connected desktop to minute previews on demand. These two actions are independent. The maintainer approved the requirements on 18 November 2026.

## Requirements

### 1. Test a saved host’s credentials

Operators need a result tied to the host and saved identity they selected.

1. When the user chooses **Test credentials** from one saved host’s menu, 1-1 shall make at most one authentication attempt against that host using that profile’s saved identity and credential.
2. While a check is pending, 1-1 shall show **Checking** for that host and coalesce repeated activation into the same attempt.
3. If the host confirms acceptance of the supplied credential, 1-1 shall show a green **Accepted** result for authentication.
4. If the host explicitly rejects the supplied credential, 1-1 shall show a red **Rejected** result.
5. If the outcome is inconclusive, 1-1 shall show gray **Unable to verify** with a reason. Examples include an unreachable host, timeout, missing saved credential, failed security negotiation, or a host that requires an interactive sign-in.
6. If the check has produced no conclusive response within the test timeout, 1-1 shall end it with the timeout outcome.
7. When a check ends, 1-1 shall close its test connection without opening, replacing, or disconnecting a desktop session.
8. When the user views a result, 1-1 shall identify the checked host and announce the outcome in text accessible without color.
9. When a check uses a stored secret, 1-1 shall keep it out of result messages, logs, exports, and saved connection documents.
10. When the host address, port, identity, or saved credential changes, 1-1 shall invalidate the previous result and discard any pending response to the old profile.

The timeout is 30 seconds. A green result confirms authentication at the time of the check. Interactive desktop access can have additional server policy requirements. The existing reachability check remains available separately.

### 2. Pause a connected display

Operators need an immediate pause for unattended output and a predictable return to typing.

1. When the user selects **Pause display** in the status bar for a supported connected session, 1-1 shall immediately enter manual minute-preview mode for that session even when automatic idle preview is off.
2. While manually paused, 1-1 shall retain the last received desktop image and request a bounded preview window from the server approximately once every 60 seconds. The pause shall persist on quiet screens until resumed or disconnected.
3. While manually paused, 1-1 shall continue processing session traffic, including clipboard transfers and clipboard Resync, on the same connection.
4. While manually paused, 1-1 shall show **Resume live** and an accessible status with the last desktop update’s age when known, within the existing status bar.
5. When the user chooses **Resume live** or sends a remote keystroke, click, scroll, or paste, 1-1 shall request live updates immediately and forward any triggering remote input once, in order.
6. While manually paused, 1-1 shall keep the pause across pointer movement alone, tab or window focus changes, and status-bar reveal or hide. If a resize requires a redraw, 1-1 shall honor the new desktop size and continue minute previews after the redraw.
7. If the host cannot pause graphics or withdraws that capability, 1-1 shall show that manual pause is unavailable and keep the display live.
8. When a paused session disconnects, 1-1 shall clear its manual override. On reconnect, the saved automatic preference shall apply as for a new session.
9. When the user operates the host menu or status-bar controls with the keyboard, 1-1 shall consume the action locally, preserving remote keyboard state and the existing status-bar auto-hide and viewport behavior.

The manual cadence prioritizes the operator’s choice. The automatic mode continues using its measured-cost policy. A forced preview can increase traffic on an otherwise quiet desktop. [Windows measurements](../windows-clearcodec-royalts-e2e/bandwidth-research.md)
