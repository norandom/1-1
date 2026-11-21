/**
 * Spec rdp-native-renderer 6.3: on Linux every web view frame repaints the
 * whole GTK window in software, so an animation that never stops keeps the
 * window busy (about 80% of the main thread on 20 November 2026). Session
 * and dashboard views, and the global styles, must not loop forever.
 */
import { describe, expect, it } from 'vitest';

const sources = import.meta.glob(
  [
    '../../components/session/**/*.{ts,tsx}',
    '../../components/dashboard/**/*.{ts,tsx}',
    '../../styles/**/*.css',
    '!**/*.test.{ts,tsx}',
    '!**/__tests__/**',
  ],
  { query: '?raw', import: 'default', eager: true },
) as Record<string, string>;

const endless = [
  { name: 'framer-motion repeat: Infinity', pattern: /repeat:\s*Infinity/ },
  { name: 'CSS infinite animation', pattern: /\binfinite\b/ },
  { name: 'Tailwind looping animation', pattern: /\banimate-(spin|pulse|ping|bounce)\b/ },
];

describe('no endless animations where the window would keep repainting', () => {
  it('reads the watched files', () => {
    const names = Object.keys(sources);
    expect(names.some((name) => name.includes('components/session/SessionCanvas.tsx'))).toBe(true);
    expect(names.some((name) => name.includes('components/dashboard/'))).toBe(true);
    expect(names.some((name) => name.endsWith('styles/globals.css'))).toBe(true);
  });

  it('finds none in the session and dashboard views or the global styles', () => {
    const found: string[] = [];
    for (const [file, text] of Object.entries(sources)) {
      text.split('\n').forEach((line, index) => {
        for (const { name, pattern } of endless) {
          if (pattern.test(line)) found.push(`${file}:${index + 1} ${name}`);
        }
      });
    }
    expect(found).toEqual([]);
  });

  it('would catch one', () => {
    expect(endless.some(({ pattern }) => pattern.test('transition={{ repeat: Infinity }}'))).toBe(true);
    expect(endless.some(({ pattern }) => pattern.test('animation: spin 1s linear infinite;'))).toBe(true);
    expect(endless.some(({ pattern }) => pattern.test('className="animate-spin"'))).toBe(true);
  });
});
