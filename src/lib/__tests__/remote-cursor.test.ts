import { describe, expect, it } from 'vitest';

import { cursorCss, cursorGeometry, needsRescale, remoteCursor, type PointerImage } from '../remote-cursor';

const arrow: PointerImage = { url: 'data:image/png;base64,AAAA', width: 32, height: 32, hotspotX: 4, hotspotY: 30 };

describe('remote cursor', () => {
  it('uses the shape as sent at one CSS pixel per remote pixel', async () => {
    expect(needsRescale(arrow, 1)).toBe(false);
    await expect(remoteCursor(arrow, 1)).resolves.toBe('url("data:image/png;base64,AAAA") 4 30, default');
  });

  it('scales size and hotspot with the canvas', () => {
    expect(cursorGeometry(arrow, 0.5)).toEqual({ width: 16, height: 16, hotspotX: 2, hotspotY: 15 });
    expect(cursorGeometry(arrow, 1.6)).toEqual({ width: 51, height: 51, hotspotX: 6, hotspotY: 48 });
  });

  it('keeps cursors within the size browsers accept', () => {
    const large: PointerImage = { ...arrow, width: 256, height: 128, hotspotX: 255, hotspotY: 0 };
    expect(cursorGeometry(large, 1)).toEqual({ width: 128, height: 64, hotspotX: 127, hotspotY: 0 });
  });

  it('keeps the hotspot inside the image', () => {
    expect(cursorCss('u', cursorGeometry({ ...arrow, hotspotX: 31, hotspotY: 31 }, 0.1))).toBe('url("u") 2 2, default');
  });
});
