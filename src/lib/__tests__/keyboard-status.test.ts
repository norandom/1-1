import { describe, expect, it } from 'vitest';
import { keyboardDescription, keyboardLabel, type ReportedLayout } from '../keyboard-status';

const layout = (overrides: Partial<ReportedLayout>): ReportedLayout => ({
  klid: 0x407, short: 'DE', name: 'German', source: 'detected', local: 'German', ...overrides,
});

describe('keyboard layout in the status bar', () => {
  it('shows the short name, or a dash when nothing was reported', () => {
    expect(keyboardLabel(layout({}))).toBe('DE');
    expect(keyboardLabel(layout({ klid: 0, short: null, source: 'serverDefault' }))).toBe('—');
  });

  it('explains every source', () => {
    expect(keyboardDescription(layout({}))).toBe('German, the local layout');
    expect(keyboardDescription(layout({ source: 'closestMatch', local: 'German (Macintosh, no dead keys)' })))
      .toBe('German, closest Windows layout for German (Macintosh, no dead keys); typed characters still match the local layout');
    expect(keyboardDescription(layout({ short: 'GB', name: 'English (UK)', source: 'override', local: null })))
      .toBe('English (UK), set for this host');
    expect(keyboardDescription(layout({ klid: 0, short: null, source: 'serverDefault' }))).toContain('remote uses its own');
    expect(keyboardDescription(layout({ klid: 0, short: null, source: 'notDetected', local: null })))
      .toContain('could not be detected');
    expect(keyboardDescription(layout({ klid: 0, short: null, source: 'notDetected', local: 'Klingon' })))
      .toContain('no Windows layout for Klingon');
  });
});

describe('keyboard section of the host editor', () => {
  it('describes the detected layout', async () => {
    const { describeLocalLayout } = await import('../keyboard-status');
    const german = { klid: 0x407, short: 'DE', name: 'German' };
    expect(describeLocalLayout({ displayName: 'German (Macintosh, no dead keys)', windows: german, matched: 'closest' }))
      .toBe('German (Macintosh, no dead keys) → German, closest match');
    expect(describeLocalLayout({ displayName: 'German', windows: german, matched: 'exact' })).toBe('German → German');
    expect(describeLocalLayout({ displayName: 'Klingon', windows: null, matched: null })).toContain('no Windows layout');
    expect(describeLocalLayout(null)).toContain('could not be detected');
  });

  it('round-trips the layout choice through the select value', async () => {
    const { layoutChoiceFrom, layoutChoiceValue } = await import('../keyboard-status');
    for (const choice of ['automatic', 'serverDefault', 0x407] as const) {
      expect(layoutChoiceFrom(layoutChoiceValue(choice))).toBe(choice);
    }
  });
});
