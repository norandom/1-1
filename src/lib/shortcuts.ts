/**
 * Shortcut-overlay catalog + platform label resolution + the shared
 * keydown matcher.
 *
 * The catalog is the displayed list as data (task 3.4 characterization:
 * extracted verbatim from `ShortcutOverlay`; which entries actually have
 * handlers is pinned by tests — see B3-13). Task 5.2 normalizes the mod
 * chords through the `Mod` sentinel: hint surfaces resolve it to ⌘ on
 * macOS and Ctrl on Windows/Linux via `usePlatformLabel` (Req 8.5), and
 * `matchKeyboardShortcut` is the single matcher both platforms share, so
 * Ctrl-based equivalents respond exactly like the macOS Cmd chords.
 */

/** Sentinel key for chords bound to the platform modifier (Cmd/Ctrl). */
export const MOD_KEY = 'Mod';
export const MAC_MOD_LABEL = '⌘';
export const OTHER_MOD_LABEL = 'Ctrl';
export type ModLabel = typeof MAC_MOD_LABEL | typeof OTHER_MOD_LABEL;

/** The desktop platforms the app ships for (windows/macos/linux builds). */
export type DesktopPlatform = 'macos' | 'windows' | 'linux' | 'unknown';

export interface ShortcutEntry {
  keys: string[];
  description: string;
}

export interface ShortcutSection {
  title: string;
  shortcuts: ShortcutEntry[];
}

export const SHORTCUT_SECTIONS: ShortcutSection[] = [
  {
    title: 'Navigation',
    shortcuts: [
      { keys: [MOD_KEY, 'K'], description: 'Quick Connect' },
      { keys: [MOD_KEY, 'N'], description: 'New Connection' },
      { keys: [MOD_KEY, ','], description: 'Settings' },
    ],
  },
  {
    title: 'Session',
    shortcuts: [
      { keys: ['Esc'], description: 'Disconnect menu' },
      { keys: ['Ctrl', 'Shift', 'P'], description: 'Performance HUD' },
      { keys: ['F11'], description: 'Fullscreen' },
    ],
  },
  {
    title: 'General',
    shortcuts: [
      { keys: [MOD_KEY, '?'], description: 'This help' },
    ],
  },
];

/** Label for the platform modifier: ⌘ on macOS, Ctrl everywhere else. */
export function modLabelForPlatform(platform: DesktopPlatform): ModLabel {
  return platform === 'macos' ? MAC_MOD_LABEL : OTHER_MOD_LABEL;
}

/** Resolve the `Mod` sentinel in a catalog chord to the platform label. */
export function resolveShortcutKeys(
  keys: readonly string[],
  modLabel: string
): string[] {
  return keys.map((key) => (key === MOD_KEY ? modLabel : key));
}

/** Actions the app actually handles (B3-13 pins the unhandled catalog rows). */
export type ShortcutAction =
  | 'quick-connect'
  | 'new-connection'
  | 'performance-hud'
  | 'shortcut-help';

/** Key event fields the matcher needs (satisfied by any KeyboardEvent). */
export interface ShortcutKeyEvent {
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  key: string;
}

/**
 * Single source for the app's keydown matching, extracted verbatim from
 * `useKeyboard` (task 5.2). The platform modifier is `metaKey || ctrlKey`,
 * so Ctrl responds on Windows/Linux and Cmd on macOS (Req 8.5). The HUD
 * chord stays literal Ctrl on every platform (the original handler binds
 * `ctrlKey` only). Match order mirrors the original handler sequence.
 */
export function matchKeyboardShortcut(e: ShortcutKeyEvent): ShortcutAction | null {
  const isMod = e.metaKey || e.ctrlKey;
  if (isMod && e.key === 'k') return 'quick-connect';
  if (isMod && e.key === 'n') return 'new-connection';
  if (e.ctrlKey && e.shiftKey && e.key === 'P') return 'performance-hud';
  if (isMod && e.shiftKey && e.key === '?') return 'shortcut-help';
  return null;
}
