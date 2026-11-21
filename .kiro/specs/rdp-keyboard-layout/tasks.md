# Keyboard layout implementation plan

Task 1 builds the data and settings (layout table, host profile). Task 2 detects the local layout per platform. Task 3 reports it at connect and shows it. Task 4 sends characters as Unicode, task 5 makes Super act as Ctrl, and task 6 verifies against the maintainer's hosts. Each leaf is one to three hours; today's behavior stays available as "Physical keys" and "Server default" throughout.

- [ ] 1. Data and settings
- [ ] 1.1 Layout table
  - `src-tauri/src/keyboard/klid.rs`: XKB layout and variant, and macOS input source IDs, to Windows KLIDs with the match kind (exact, closest, unknown), plus a short name ("DE") and a display name. Unit tests for exact, closest (Mac and no-dead-keys variants) and unknown entries.
  - _Requirements: 1.1, 1.2, 7.3_
- [ ] 1.2 Keyboard section in the host profile
  - `KeyboardProfile { layout: Automatic | ServerDefault | Fixed(klid), keys: Automatic | Physical, super_as_ctrl }` in `HostProfile` with serde defaults (Automatic, Automatic, on); TypeScript types and normalization; import, export and duplicate keep it. Tests for defaults and old profiles.
  - _Requirements: 1.4, 4.1, 4.3, 7.3_
- [ ] 1.3 Keyboard section in the host editor
  - `HostProfileFields`: Layout (Automatic, a list of common layouts, Server default), Keys (Automatic, Physical keys, with the note about servers without Unicode input), Super as Ctrl. Shows the detected local layout (`detect_keyboard_layout`, from 2.x).
  - _Depends: 1.2_
  - _Requirements: 4.1, 4.2, 5.2, 7.1_

- [ ] 2. Detect the local layout
- [ ] 2.1 Wayland keymap
  - `src-tauri/src/keyboard/detect.rs`: bind `wl_seat` and `wl_keyboard` on a queue over GDK's connection, read the keymap's `xkb_symbols` include line and the active group from `wl_keyboard.modifiers`; dispatch from GTK's main loop. Parser tests with real Hyprland keymaps (one layout, two layouts and a switch, variants).
  - _Depends: 1.1_
  - _Requirements: 6.1_
- [ ] 2.2 X11, Windows and macOS
  - X11: `setxkbmap -query`. Windows: `GetKeyboardLayoutNameW`. macOS: `TISCopyCurrentKeyboardInputSource` and its input source ID. Each falls back to "not detected".
  - _Depends: 1.1_
  - _Requirements: 6.2, 6.3, 6.4, 6.5_

- [ ] 3. Report and show the layout
- [ ] 3.1 Report at connect
  - Resolve `ReportedLayout { klid, name, source }` from the host's setting and the detection; `build_connector_config` sets `keyboard_layout`; log once; store it in `SessionInfo.keyboard`. Tests for every source (detected, closest, override, server default, not detected).
  - _Depends: 1.2, 2.1_
  - _Requirements: 1.1, 1.2, 1.3, 4.2_
- [ ] 3.2 Status bar and HUD
  - A compact status bar item (for example `DE`) for the connected session, with the full name and source on hover, hidden without a session; one HUD line with the layout and key mode. Frontend tests for the item's text and tooltip.
  - _Depends: 3.1_
  - _Requirements: 5.1, 5.3_

- [ ] 4. Characters as characters
- [ ] 4.1 Key decision
  - `src/lib/keyboard-input.ts`: scancode, text or nothing per key press (printable grapheme, AltGr and Option results, dead keys, named keys, Ctrl and Super chords, Physical mode). Unit tests for each case.
  - _Requirements: 2.1, 2.2_
- [ ] 4.2 Unicode events
  - `send_key_event` text variant and `SessionCommand::SendText`: UTF-16 code units as Unicode press and release in one batch, ordered with other input. Key-up bookkeeping in `canvas-keyboard.ts` for keys sent as text; release-all on blur unchanged. Tests for encoding (BMP, surrogate pairs) and for releases after mixed paths.
  - _Depends: 4.1_
  - _Requirements: 2.1, 2.4_
- [ ] 4.3 Input methods
  - The hidden composition field: focus on `compositionstart`, forward `compositionend` text, return focus to the canvas; `data-remote-input` counts as remote. Checked by hand with a compose sequence and an input method.
  - _Depends: 4.2_
  - _Requirements: 2.3_

- [ ] 5. Super as Ctrl
- [ ] 5.1 Chord rewrite and tap
  - In `canvas-keyboard.ts`: hold Super back; a chord sends Ctrl down, the key and Ctrl up; a lone tap sends the Windows key; blur releases what was sent; off when the host's `super_as_ctrl` is off; 1-1's own shortcuts unchanged. Tests for chord, tap, repeat, blur and the setting.
  - _Depends: 4.1_
  - _Requirements: 3.1, 3.2, 3.3_

- [ ] 6. Verify with the maintainer
- [ ] 6.1 Windows and xrdp
  - On the reference machine (Hyprland, German Macintosh) against the Windows host and an xrdp host, Automatic and Physical keys: the layout the remote starts with, a line of symbols in an editor, Super+C/V, Ctrl+Z, vim in a terminal, the status bar item. Record in `validation.md`.
  - _Depends: 3.2, 4.3, 5.1_
  - _Requirements: 7.2_
