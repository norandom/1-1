import { describe, expect, it } from 'vitest';

import {
  SHORTCUT_SECTIONS,
  MOD_KEY,
  MAC_MOD_LABEL,
  OTHER_MOD_LABEL,
  modLabelForPlatform,
  resolveShortcutKeys,
  matchKeyboardShortcut,
} from '../shortcuts';

const descriptions = () =>
  SHORTCUT_SECTIONS.flatMap((section) => section.shortcuts.map((s) => s.description));

describe('SHORTCUT_SECTIONS', () => {
  it('catalogs the three overlay sections in display order', () => {
    expect(SHORTCUT_SECTIONS.map((s) => s.title)).toEqual([
      'Navigation',
      'Session',
      'General',
    ]);
  });

  it('lists the seven advertised shortcuts with platform-neutral chords', () => {
    expect(descriptions()).toEqual([
      'Quick Connect',
      'New Connection',
      'Settings',
      'Disconnect menu',
      'Performance HUD',
      'Fullscreen',
      'This help',
    ]);
    for (const section of SHORTCUT_SECTIONS) {
      for (const shortcut of section.shortcuts) {
        expect(shortcut.keys.length).toBeGreaterThan(0);
      }
    }
    // Task 5.2 (Req 8.5): mod chords carry the `Mod` sentinel resolved per
    // platform (⌘ on macOS, Ctrl on Windows/Linux); the catalog never
    // bakes in a platform-specific modifier.
    expect(SHORTCUT_SECTIONS[0].shortcuts[0].keys).toEqual([MOD_KEY, 'K']);
    expect(SHORTCUT_SECTIONS[2].shortcuts[0].keys).toEqual([MOD_KEY, '?']);
    for (const section of SHORTCUT_SECTIONS) {
      for (const shortcut of section.shortcuts) {
        expect(shortcut.keys).not.toContain('\u2318');
        expect(shortcut.keys).not.toContain('Cmd');
      }
    }
    // The HUD chord is literal Ctrl on every platform: the handler binds
    // e.ctrlKey only (see matchKeyboardShortcut), so macOS shows Ctrl too.
    expect(SHORTCUT_SECTIONS[1].shortcuts[1].keys).toEqual(['Ctrl', 'Shift', 'P']);
  });

  it('advertises Settings (Mod+,) and Fullscreen (F11) which have no handlers_defect_b3_13', () => {
    // B3-13: the overlay advertises Mod+, Settings and F11 Fullscreen, but
    // no keydown handler or menu accelerator for either exists in the app
    // (matchKeyboardShortcut handles only K/N/Ctrl+Shift+P/?; fullscreen is
    // a toolbar button only). Pinned as a defect; a fix must either wire or
    // remove these entries, which fails this test.
    const advertised = descriptions();
    expect(advertised).toContain('Settings');
    expect(advertised).toContain('Fullscreen');
  });
});

describe('modLabelForPlatform', () => {
  it('labels macOS with ⌘ and every other platform with Ctrl (Req 8.5)', () => {
    expect(modLabelForPlatform('macos')).toBe('\u2318');
    expect(modLabelForPlatform('windows')).toBe('Ctrl');
    expect(modLabelForPlatform('linux')).toBe('Ctrl');
    expect(modLabelForPlatform('unknown')).toBe('Ctrl');
  });
});

describe('resolveShortcutKeys', () => {
  it('resolves Mod to Ctrl for Windows/Linux hints (Req 8.5)', () => {
    expect(resolveShortcutKeys([MOD_KEY, 'K'], OTHER_MOD_LABEL)).toEqual(['Ctrl', 'K']);
    expect(resolveShortcutKeys([MOD_KEY, ','], 'Ctrl')).toEqual(['Ctrl', ',']);
  });

  it('resolves Mod to ⌘ for macOS hints', () => {
    expect(resolveShortcutKeys([MOD_KEY, '?'], MAC_MOD_LABEL)).toEqual(['\u2318', '?']);
    expect(resolveShortcutKeys([MOD_KEY, 'N'], '\u2318')).toEqual(['\u2318', 'N']);
  });

  it('passes non-modifier keys through untouched', () => {
    expect(resolveShortcutKeys(['Ctrl', 'Shift', 'P'], 'Ctrl')).toEqual([
      'Ctrl',
      'Shift',
      'P',
    ]);
    expect(resolveShortcutKeys(['Esc'], '\u2318')).toEqual(['Esc']);
    expect(resolveShortcutKeys(['F11'], 'Ctrl')).toEqual(['F11']);
  });
});

describe('matchKeyboardShortcut', () => {
  // Req 8.5: the app's handlers respond to Ctrl-based equivalents on
  // Windows/Linux and to Cmd on macOS — the matcher treats metaKey and
  // ctrlKey as the same platform modifier.
  it('matches Ctrl and Cmd equivalently for mod chords', () => {
    expect(
      matchKeyboardShortcut({ metaKey: false, ctrlKey: true, shiftKey: false, key: 'k' })
    ).toBe('quick-connect');
    expect(
      matchKeyboardShortcut({ metaKey: true, ctrlKey: false, shiftKey: false, key: 'k' })
    ).toBe('quick-connect');
    expect(
      matchKeyboardShortcut({ metaKey: false, ctrlKey: true, shiftKey: false, key: 'n' })
    ).toBe('new-connection');
    expect(
      matchKeyboardShortcut({ metaKey: true, ctrlKey: false, shiftKey: false, key: 'n' })
    ).toBe('new-connection');
    expect(
      matchKeyboardShortcut({ metaKey: false, ctrlKey: true, shiftKey: true, key: '?' })
    ).toBe('shortcut-help');
    expect(
      matchKeyboardShortcut({ metaKey: true, ctrlKey: false, shiftKey: true, key: '?' })
    ).toBe('shortcut-help');
  });

  it('matches the literal Ctrl+Shift+P HUD chord on every platform', () => {
    expect(
      matchKeyboardShortcut({ metaKey: false, ctrlKey: true, shiftKey: true, key: 'P' })
    ).toBe('performance-hud');
    expect(
      matchKeyboardShortcut({ metaKey: true, ctrlKey: false, shiftKey: true, key: 'P' })
    ).toBeNull();
  });

  it('ignores chords without the platform modifier', () => {
    expect(
      matchKeyboardShortcut({ metaKey: false, ctrlKey: false, shiftKey: false, key: 'k' })
    ).toBeNull();
    expect(
      matchKeyboardShortcut({ metaKey: false, ctrlKey: false, shiftKey: true, key: '?' })
    ).toBeNull();
    expect(
      matchKeyboardShortcut({ metaKey: false, ctrlKey: true, shiftKey: false, key: 'x' })
    ).toBeNull();
  });

  it('keeps the original handler match order (characterization)', () => {
    // Ctrl+Shift+K still lands on quick-connect exactly as the useKeyboard
    // sequence always has: the k check precedes the HUD chord check.
    expect(
      matchKeyboardShortcut({ metaKey: false, ctrlKey: true, shiftKey: true, key: 'k' })
    ).toBe('quick-connect');
  });
});
