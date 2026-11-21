# Design: keyboard layout

## Overview

Three changes work together:

1. **Report the layout.** At connect, the connector's `keyboard_layout` (today `0`, `rdp/client.rs`) carries a Windows keyboard layout identifier (KLID) resolved from the host's keyboard setting and the detected local layout. Windows starts the session with it; xrdp picks its keymap from it.
2. **Send characters as characters.** The page decides per key press: a printable character goes as a Unicode keyboard event (`FastPathInputEvent::UnicodeKeyboardEvent`, one UTF-16 code unit per event, supported by `ironrdp-pdu` 0.11), everything else as today's scancode.
3. **Super as Ctrl.** The page rewrites Super chords to Ctrl chords; Super tapped alone stays the Windows key.

The first makes shortcuts and the remote's own idea of the layout right; the second makes every typed character right even where no Windows layout matches the local one (Mac variants); the third is the Mac habit.

## Components

### Local layout detection (`src-tauri/src/keyboard/detect.rs`, new)

Returns `LocalLayout { xkb_layout, xkb_variant, display_name }` or a platform equivalent, cached in app state and refreshed when it changes.

- **Linux, Wayland:** bind `wl_seat` and `wl_keyboard` on our own event queue over GDK's connection (as `native_layer.rs` does). The compositor sends the keymap as an XKB text file (`wl_keyboard.keymap`, a file descriptor); its `xkb_symbols` include line names the layouts and variants in group order (for example `pc+de(mac)+us:2+inet(evdev)`), and `wl_keyboard.modifiers` carries the active group. Parse the include line; take the active group. Hyprland sends both on bind and on every layout switch. The queue is dispatched from GTK's main loop (a 500 ms timer, like the layer's), so the cache follows switches.
- **Linux, X11:** run `setxkbmap -query` once per connect and take its first `layout` and `variant`; if the tool is missing, detection fails (requirement 1.3).
- **Windows:** `GetKeyboardLayoutNameW` on the window's thread gives the KLID directly (`windows` crate, feature `Win32_UI_Input_KeyboardAndMouse`).
- **macOS:** `TISCopyCurrentKeyboardInputSource` and `kTISPropertyInputSourceID` (Carbon, via `objc2-core-foundation`, already in the lockfile) give an ID such as `com.apple.keylayout.German`.

### Layout identifiers (`src-tauri/src/keyboard/klid.rs`, new)

A table from XKB layout and variant, and from macOS input source IDs, to KLIDs, with the source of the match:

- Exact: `us` → `00000409`, `de` → `00000407`, `gb` → `00000809`, `fr` → `0000040C`, `ch(de)` → `00000807`, `ch(fr)` → `0000100C`, `at` → `00000C07`, `es` → `0000040A`, `it` → `00000410`, `nl` → `00000413`, `se` → `0000041D`, `no` → `00000414`, `dk` → `00000406`, `fi` → `0000040B`, `pl` → `00000415`, `pt` → `00000816`, `br` → `00000416`, `us(intl)` → `00020409`, `us(dvorak)` → `00010409`, and the macOS IDs for the same languages.
- Closest: variants without a Windows layout (`mac`, `nodeadkeys`, `mac_nodeadkeys`, …) map to their base layout, marked `ClosestMatch`.
- Unknown: `None`, marked `NotDetected`.

The table is data with unit tests (requirement 7.3); adding a layout is one line.

### Host profile (`store/connections.rs`)

```rust
pub struct KeyboardProfile {
    pub layout: LayoutChoice,  // Automatic (default) | ServerDefault | Fixed(Klid)
    pub keys: KeyMode,         // Automatic (default) | Physical
    pub super_as_ctrl: bool,   // default true
}
```

`HostProfile` gains `keyboard: KeyboardProfile` with `#[serde(default)]`, so older profiles read as Automatic (requirement 4.3). Import, export and duplicate copy it with the rest of the profile.

### Connect (`rdp/client.rs`)

`build_connector_config` takes the resolved layout: `Automatic` → the detected KLID or `0`; `ServerDefault` → `0`; `Fixed(klid)` → that KLID. The resolution (`ReportedLayout { klid, name, source }`, source one of `Detected`, `ClosestMatch`, `Override`, `ServerDefault`, `NotDetected`) is logged once and stored in the session for the status bar and the HUD (requirement 5).

### Key events from the page (`src/lib/keyboard-input.ts`, new; `canvas-keyboard.ts`, `useRdpSession.ts`)

A pure function decides for each `keydown`:

```ts
type KeyAction =
  | { kind: 'scancode'; code: string }   // today's path
  | { kind: 'text'; text: string }       // Unicode events
  | { kind: 'none' };                    // dead key, or held for Super
```

- **Text** when the key mode is Automatic, `event.key` is a single printable grapheme (not a named key such as `Enter`, not `Dead`), and neither Ctrl nor Super is held, except that AltGr (`getModifierState('AltGraph')`, which browsers on Windows also report as Ctrl+Alt) counts as producing text. Option combinations on a Mac layout arrive as the produced character (`@` for Option+L) and therefore go as text.
- **Dead keys:** `event.key === 'Dead'` sends nothing; the next key press carries the composed character as its `key` and goes as text.
- **Input methods:** WebKit delivers composition only to editable elements. The canvas keeps focus for keys; for composition the design adds a visually hidden `<textarea data-remote-input>` that receives focus during composition (`compositionstart` on the window) and forwards `compositionend` data as text, then returns focus to the canvas. `isLocalInputTarget` treats `data-remote-input` as remote. This path is verified by hand (requirement 2.3) and can be cut without affecting the rest.
- **Key up:** a key whose down went as text sends no up (the Unicode event is sent as press and release); the existing `forwardedKeys` set records which path each held key took, so releases and the release-all on blur stay correct (requirement 2.4). Key repeat sends the text again.

Backend: `send_key_event` gains a text variant; `SessionCommand::SendText { text }` encodes each UTF-16 code unit as `UnicodeKeyboardEvent` press then release (surrogate pairs as two units), in one fast-path batch, ordered with other input as today.

### Super as Ctrl (`canvas-keyboard.ts`)

With `super_as_ctrl` on:

- Super down: held back, nothing sent.
- Another key while Super is held: send Ctrl down (once per chord), then that key as a scancode (never as text: it is a shortcut).
- Super up: if a chord happened, send Ctrl up; otherwise send the Windows key down and up (Start menu).
- Blur while held: release whatever was sent.

The existing deferral logic for modifiers during a paused display (`deferred` in `subscribeCanvasKeyboard`) is the model; this is a second, separate deferral with its own tests.

### Showing it (requirement 5)

- `SessionInfo` gains `keyboard: { klid, name, source, keyMode, superAsCtrl }`.
- **Status bar:** a compact item next to the clipboard status, for example `DE`, from the layout's short name; the tooltip gives the full name and the source ("German, closest match for German (Macintosh)", "US, set for this host", "none reported: server default"). Hidden while no session is connected (requirement 5.3).
- **HUD:** one line with the same text and the key mode.
- **Host profile:** the keyboard section shows the detected local layout ("Detected: German (Macintosh) → German") next to the three choices (requirement 5.2); the detection is read through a `detect_keyboard_layout` command.

## Error handling

| Situation | Behavior |
| --- | --- |
| Detection fails or the layout is unknown | Report `0`, source `NotDetected`, logged once; typed text is still correct through Unicode |
| `setxkbmap` missing (X11) | As above |
| Server ignores Unicode events | Characters do not arrive; the host's keyboard section explains "Physical keys" (requirement 7.1) |
| A text event's code unit is a lone surrogate | Dropped, logged at debug |

## Testing

- **Rust:** the KLID table (exact, closest, unknown); the XKB include-line parser with real Hyprland keymaps (single layout, two layouts with the active group, variants); `KeyboardProfile` defaults and old profiles; `SendText` encoding (BMP, surrogate pair, press/release order).
- **Frontend:** the key decision for printable keys, AltGr and Option characters, dead keys, Ctrl and Super chords, named keys, and Physical mode; Super as Ctrl (chord, tap, blur); releases after mixed paths; the status bar item and its tooltip.
- **By hand (requirement 7.2):** on the reference machine with German (Macintosh), against the maintainer's Windows host and a Linux host with xrdp, both key modes: the layout the remote session starts with, a line of symbols, Super+C/V, Ctrl+Z, vim in a terminal; recorded in `validation.md`.

## Requirements traceability

| Requirement | Design |
| --- | --- |
| 1.1–1.4 | Detection, KLID table, connect resolution, profile defaults |
| 2.1–2.4 | Key decision, dead keys, composition proxy, key-up bookkeeping, `SendText` |
| 3.1–3.3 | Super as Ctrl deferral |
| 4.1–4.3 | `KeyboardProfile`, host keyboard section, serde defaults |
| 5.1–5.3 | `SessionInfo.keyboard`, status bar item, HUD line, detected layout in the profile |
| 6.1–6.5 | Per-platform detection |
| 7.1–7.3 | Physical keys note, manual verification, automated tests |
