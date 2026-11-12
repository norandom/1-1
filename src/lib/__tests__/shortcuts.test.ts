import { describe, expect, it } from 'vitest';

import { SHORTCUT_SECTIONS } from '../shortcuts';

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

  it('lists the seven advertised shortcuts with key chords', () => {
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
    expect(SHORTCUT_SECTIONS[2].shortcuts[0].keys).toEqual(['\u2318', '?']);
    expect(SHORTCUT_SECTIONS[1].shortcuts[1].keys).toEqual(['Ctrl', 'Shift', 'P']);
  });

  it('advertises Settings (⌘,) and Fullscreen (F11) which have no handlers_defect_b3_13', () => {
    // B3-13: the overlay advertises ⌘, Settings and F11 Fullscreen, but no
    // keydown handler or menu accelerator for either exists in the app
    // (useKeyboard handles only K/N/Ctrl+Shift+P/?; fullscreen is a toolbar
    // button only). Pinned as a defect; a fix must either wire or remove
    // these entries, which fails this test.
    const advertised = descriptions();
    expect(advertised).toContain('Settings');
    expect(advertised).toContain('Fullscreen');
  });
});
