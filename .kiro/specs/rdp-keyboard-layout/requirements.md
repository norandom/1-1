# Keyboard layout

## Introduction

1-1 sends every key as its physical position (a scancode) and reports no keyboard layout when it connects (`keyboard_layout: 0`). The remote session then reads the keys through its own layout. Letters usually match, but symbols do not: `@ | { } [ ] ~`, the key next to left Shift, and what Option or AltGr combinations produce. On QWERTZ and AZERTY layouts even shortcuts move (Ctrl+Z becomes Ctrl+Y on a US session). Linux hosts are worse: xrdp picks its keymap from the layout the client reports, so a client that reports none, or US, gets US. The maintainer sees this with xfreerdp on Linux today ("I am always US").

The maintainer approved these requirements on 21 November 2026. The maintainer types on a Mac layout under Linux and uses another layout on Windows. What they want: by default, the remote session uses the layout the client uses, on Windows and on Linux hosts, without setup on either side. Users may override that per host, but nobody has to.

## Boundary

- **In scope:** detecting the local layout, reporting it to the server at connect, sending typed characters so they match the local layout, Super as Ctrl, per-host overrides, and the platforms 1-1 ships on (Linux, Windows, macOS).
- **Out of scope:** changing the remote layout during a running session (a reconnect applies a changed local layout), an input method editor's candidate window, and remapping keys other than Super.

## Requirements

### 1. Automatic: the remote uses the client's layout (default)

1. When a session connects with the keyboard setting on Automatic, 1-1 shall detect the active local keyboard layout and report it to the server as a Windows keyboard layout identifier (KLID) in the connection's client data, so that Windows and xrdp start the session with that layout.
2. Where the local layout has no Windows identifier of its own, such as a Mac variant of a layout, 1-1 shall report the closest Windows layout for the same language and country (for example German for German Macintosh), and typed characters shall still match the local layout (requirement 2).
3. If 1-1 cannot detect the local layout, it shall report nothing (as today), keep the session usable, and say so in the log and in the session's keyboard details (requirement 5).
4. Automatic shall be the default for new hosts and for existing hosts that have no keyboard setting.

### 2. Typed characters match the local layout

1. In Automatic, when a key press produces a printable character on the local layout, including Option or AltGr combinations and the result of a dead key or compose sequence, 1-1 shall send that character as a Unicode keyboard event, so the remote receives exactly what the local layout produced, whatever layout the remote session uses.
2. Keys that do not produce a printable character, and chords with Ctrl, Alt (when it does not produce a character), Super or the Windows key, shall go as scancodes, so shortcuts, editors such as vim, terminals and games behave as they do today.
3. Text from an input method (composition) shall arrive at the remote as the composed characters.
4. Key repeat, press and release order, and releasing every held key when focus leaves the session shall behave as they do today.

### 3. Super as Ctrl

1. In Automatic, a chord of Super with another key shall reach the remote as the same chord with Ctrl, so Super+C, Super+V, Super+Z and similar shortcuts work as on a Mac.
2. Super pressed and released alone shall still reach the remote as the Windows key, so the Start menu stays reachable.
3. 1-1's own shortcuts shall keep working as today.

### 4. Overrides per host, not as default

1. Each host shall offer, in its profile, a keyboard section with:
   - **Layout:** Automatic (default), a specific layout from a list of common Windows layouts, or "Server default" (report nothing, today's behavior).
   - **Keys:** Automatic (default: characters as characters, everything else as keys) or "Physical keys" (every key as a scancode, today's behavior).
   - **Super as Ctrl:** on by default in Automatic, switchable off.
2. An override shall apply at the next connection of that host and leave every other host on its own setting.
3. Imported and duplicated hosts shall keep their keyboard settings; hosts from older versions shall read as Automatic.

### 5. Visible and explainable

1. The session's details (the Performance HUD or the session information) shall show the layout that was reported, where it came from (detected, closest match, override, or none) and the key mode in use.
2. The host's keyboard section shall show the currently detected local layout next to the choice.
3. While a session is connected, the status bar shall show the keyboard layout reported for it (for example "DE" with the full name and its source on hover), or that none was reported (amendment, 21 November 2026, at the maintainer's request).

### 6. Platforms

1. Linux, Wayland: 1-1 shall read the compositor's keymap (layout and variant) for the detection; Hyprland is the reference.
2. Linux, X11: 1-1 shall read the X keyboard layout.
3. Windows: 1-1 shall use the active input locale of its window.
4. macOS: 1-1 shall use the current keyboard input source.
5. Where a platform's detection is not available, requirement 1.3 applies.

### 7. Compatibility and verification

1. A server that ignores Unicode keyboard events shall be usable with "Physical keys"; the host's keyboard section shall say so.
2. Verification on the reference machine (Linux, Hyprland, German Macintosh layout) against the maintainer's Windows host and a Linux host with xrdp shall record, for both Automatic and Physical keys: the layout the remote session starts with, a line of symbols typed in a text editor, Super+C/V, Ctrl+Z, and vim in a terminal.
3. Automated tests shall cover the mapping from local layouts to Windows identifiers, the choice between Unicode and scancode for a key press, Super as Ctrl, and reading old host profiles.
