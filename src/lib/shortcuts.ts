/**
 * Shortcut-overlay catalog extracted from `ShortcutOverlay`
 * (task 3.4 characterization): the displayed list as data, rewired into
 * the component verbatim. Which entries actually have handlers is pinned
 * by tests (see B3-13: two advertised shortcuts have no handler).
 */

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
      { keys: ['\u2318', 'K'], description: 'Quick Connect' },
      { keys: ['\u2318', 'N'], description: 'New Connection' },
      { keys: ['\u2318', ','], description: 'Settings' },
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
      { keys: ['\u2318', '?'], description: 'This help' },
    ],
  },
];
