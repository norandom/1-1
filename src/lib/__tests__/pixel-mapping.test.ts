import { describe, expect, it } from 'vitest';
import { compositeFrameRects, layoutSurface, type DisplayMode } from '../canvas-compositor';
import { isDisplayModeShortcut, mapMouseEvent, releaseForwardedKeys } from '../input-mapper';

it('reserves only the documented display shortcut and releases every forwarded modifier before local focus', () => {
  expect(isDisplayModeShortcut({ code: 'Tab', ctrlKey: false, altKey: false, shiftKey: false })).toBe(false);
  expect(isDisplayModeShortcut({ code: 'KeyP', ctrlKey: true, altKey: true, shiftKey: false })).toBe(false);
  expect(isDisplayModeShortcut({ code: 'KeyP', ctrlKey: true, altKey: true, shiftKey: true })).toBe(true);
  const held = new Set(['ControlLeft', 'AltRight', 'ShiftLeft']);
  const released: string[] = [];
  releaseForwardedKeys(held, (code) => released.push(code));
  expect(released).toEqual(['ControlLeft', 'AltRight', 'ShiftLeft']);
  expect(held.size).toBe(0);
  releaseForwardedKeys(held, (code) => released.push(code));
  expect(released).toHaveLength(3);
});

describe.each([1, 1.25, 1.5, 2])('pixel mapping at DPR %s', (dpr) => {
  for (const mode of ['native', 'fit'] as DisplayMode[]) {
    it(`${mode}: preserves text RGBA through full and dirty updates`, () => {
      // Fixed 3x5 bitmap letter H, including colored antialias samples.
      const glyph = ['101', '101', '111', '101', '101'].join('');
      const pixels = Uint8Array.from([...glyph].flatMap((v) => v === '1' ? [23, 61, 97, 255] : [240, 241, 242, 255]));
      const surface = { width: 3, height: 5, pixels: new Uint8ClampedArray(60), dirty: null };
      compositeFrameRects(surface, [{ x: 0, y: 0, width: 3, height: 5, pixels }]);
      expect([...surface.pixels]).toEqual([...pixels]);
      const layout = layoutSurface({ width: 101, height: 79, left: 13.3, top: 29.7, scrollLeft: 0, scrollTop: 0 }, 3, 5, mode, dpr);
      expect(layout.width / layout.height).toBeCloseTo(3 / 5, 12);
      if (mode === 'native') {
        expect(layout.width * dpr).toBe(3);
        expect(layout.height * dpr).toBe(5);
      }
      const patch = Uint8Array.from([120, 54, 12, 255, 210, 180, 160, 255]);
      compositeFrameRects(surface, [{ x: 1, y: 1, width: 1, height: 2, pixels: patch }]);
      const expected = pixels.slice();
      expected.set(patch.subarray(0, 4), 16);
      expected.set(patch.subarray(4), 28);
      expect([...surface.pixels]).toEqual([...expected]);
      expect([surface.width, surface.height]).toEqual([3, 5]);
    });

    it(`${mode}: shares scrolled content coordinates for pointer and wheel without double scaling`, () => {
      for (const scroll of [0, 37.2, 111]) {
        const viewport = { width: 501, height: 301, left: 13.3, top: 29.7, scrollLeft: scroll, scrollTop: scroll / 2 };
        const layout = layoutSurface(viewport, 1920, 1080, mode, dpr);
        const left = viewport.left + layout.left - scroll;
        const top = viewport.top + layout.top - scroll / 2;
        if (mode === 'native') {
          expect(left * dpr).toBeCloseTo(Math.round(left * dpr), 10);
          expect(top * dpr).toBeCloseTo(Math.round(top * dpr), 10);
          expect(layout.width * dpr).toBe(1920);
        }
        const bounds = { rect: { left, top, width: layout.width, height: layout.height } as DOMRect, surfaceWidth: 1920, surfaceHeight: 1080 };
        for (const eventType of ['move', 'down', 'up', 'scroll'] as const) {
          const event = { clientX: left + 437 * layout.width / 1920, clientY: top + 217 * layout.height / 1080, button: 0 } as MouseEvent;
          expect(mapMouseEvent(event, eventType, bounds)).toMatchObject({ x: 437, y: 217, eventType });
        }
      }
    });
  }
});
