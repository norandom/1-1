import { describe, expect, it } from 'vitest';

import { nextThemeInCycle, resolveTheme } from '../theme';

describe('resolveTheme', () => {
  it("resolves 'system' from the OS color scheme", () => {
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('system', false)).toBe('light');
  });

  it("passes explicit 'dark' and 'light' through unchanged", () => {
    expect(resolveTheme('dark', true)).toBe('dark');
    expect(resolveTheme('dark', false)).toBe('dark');
    expect(resolveTheme('light', true)).toBe('light');
    expect(resolveTheme('light', false)).toBe('light');
  });
});

describe('nextThemeInCycle', () => {
  it('cycles dark → light → system → dark', () => {
    expect(nextThemeInCycle('dark')).toBe('light');
    expect(nextThemeInCycle('light')).toBe('system');
    expect(nextThemeInCycle('system')).toBe('dark');
  });
});
