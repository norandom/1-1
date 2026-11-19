import { describe, expect, it } from 'vitest';
import { wantsBrowserMenu } from '../browser-menu';

/** Minimal stand-ins: `closest` is all the check uses. */
function element(matches: boolean): EventTarget {
  const e = { closest: () => (matches ? e : null) };
  return e as unknown as EventTarget;
}

describe('the WebView context menu', () => {
  it('is suppressed on the app surface, including hosts in the tree', () => {
    expect(wantsBrowserMenu(element(false), '')).toBe(false);
    expect(wantsBrowserMenu(null, '')).toBe(false);
  });
  it('stays for text fields and selected text', () => {
    expect(wantsBrowserMenu(element(true), '')).toBe(true);
    expect(wantsBrowserMenu(element(false), 'copied text')).toBe(true);
    expect(wantsBrowserMenu(element(false), '   ')).toBe(false);
  });
});
