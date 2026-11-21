# Keyboard layout validation

## 21 November 2026: first checks by the maintainer

Reference machine: Linux, Hyprland, layouts German (Macintosh, no dead keys) and English (US). Builds from master up to `db5639e`, then `df4662e`.

- The remote Windows session now starts with the client's layout ("the layout switches"): 1-1 reports German (`00000407`), the closest Windows layout for German Macintosh.
- The host settings show the Keyboard section (Layout, Keys, Super as Ctrl, detected layout) after Shared folders.

Later the same day the maintainer reported "all works" after using the build in their sessions to the Windows host; task 6.1 is closed on that report. No separate result was given for an xrdp host or for Physical keys mode. Input methods (task 4.3) are deferred.
