# Saved-host credential check and manual display pause

## Problem and desired outcome

Operators need to know whether an RDP host accepts a saved credential before opening a desktop. An open port establishes reachability only. Operators also need to slow a busy remote display as soon as they leave a coding or agent session unattended. Automatic idle preview currently waits five minutes and considers traffic costs.

## Scope and boundary candidates

The host menu owns **Test credentials** for a single saved connection. Its result describes authentication at the time of the test. The status bar owns **Pause display** and **Resume live** for the active session. These actions can ship independently.

## Proposed behavior for review

One manual credential check uses the selected host’s saved identity, finishes before opening a desktop, and reports any inconclusive result with a reason. A manual display pause takes effect immediately, keeps about one preview per minute, and lasts until explicit resume, deliberate remote input, or the end of the connection. A focus change alone keeps a manually paused desktop paused.

## Upstream and downstream

These actions use the existing per-host credential store, session lifecycle, graphics-suppression capability, and accessible host menu and status bar. The editor’s reachability check and the automatic idle-preview preference have their own existing behavior. Shared folders and copy or sync commands have separate acceptance criteria. [Existing controls and interoperability tasks](../windows-clearcodec-royalts-e2e/tasks.md)
