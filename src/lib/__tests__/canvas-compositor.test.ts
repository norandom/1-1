import { describe, expect, it } from 'vitest';

import {
  compositeFrameRects,
  fitSurfaceToContainer,
  unionDirtyRect,
  type CompositingSurface,
} from '../canvas-compositor';

function makeSurface(width: number, height: number): CompositingSurface {
  return {
    width,
    height,
    pixels: new Uint8ClampedArray(width * height * 4),
    dirty: null,
  };
}

describe('fitSurfaceToContainer', () => {
  it('scales to the limiting axis (letterbox fit)', () => {
    // 16:9 surface in a 4:3 container: width-bound, vertical letterbox bars.
    expect(fitSurfaceToContainer(800, 600, 1920, 1080)).toEqual({
      width: 800,
      height: 450,
    });
    // Taller-than-container surface: height-bound.
    expect(fitSurfaceToContainer(100, 100, 100, 1000)).toEqual({
      width: 10,
      height: 100,
    });
  });

  it('upscales when the container is larger than the surface', () => {
    expect(fitSurfaceToContainer(2000, 2000, 1000, 500)).toEqual({
      width: 2000,
      height: 1000,
    });
  });

  it('floors fractional scales and never returns a zero dimension', () => {
    expect(fitSurfaceToContainer(100, 100, 3, 2)).toEqual({ width: 100, height: 66 });
    expect(fitSurfaceToContainer(1, 1, 1000, 1000)).toEqual({ width: 1, height: 1 });
  });

  it('falls back to natural size on degenerate inputs', () => {
    expect(fitSurfaceToContainer(0, 600, 1920, 1080)).toEqual({
      width: 1920,
      height: 1080,
    });
    expect(fitSurfaceToContainer(800, 0, 0, 500)).toEqual({ width: 1, height: 500 });
  });
});

describe('unionDirtyRect', () => {
  it('returns the next rect when there is no current dirty region', () => {
    const next = { x: 1, y: 2, width: 3, height: 4 };
    expect(unionDirtyRect(null, next)).toBe(next);
  });

  it('unions overlapping and disjoint rects into their bounding box', () => {
    expect(
      unionDirtyRect({ x: 0, y: 0, width: 10, height: 10 }, { x: 5, y: 5, width: 10, height: 10 })
    ).toEqual({ x: 0, y: 0, width: 15, height: 15 });
    expect(
      unionDirtyRect({ x: 0, y: 0, width: 2, height: 2 }, { x: 8, y: 8, width: 2, height: 2 })
    ).toEqual({ x: 0, y: 0, width: 10, height: 10 });
  });
});

describe('compositeFrameRects', () => {
  it('replaces the whole surface when the rect exactly covers it', () => {
    const surface = makeSurface(2, 1);
    const pixels = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);

    compositeFrameRects(surface, [
      { x: 0, y: 0, width: 2, height: 1, pixels },
    ]);

    expect(Array.from(surface.pixels)).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
    expect(surface.dirty).toEqual({ x: 0, y: 0, width: 2, height: 1 });
  });

  it('writes a partial rect row by row at its offset', () => {
    const surface = makeSurface(4, 2); // 2 rows of 4 px
    // 2x1 rect at (1,0): one row of 2 pixels.
    compositeFrameRects(surface, [
      { x: 1, y: 0, width: 2, height: 1, pixels: new Uint8Array([9, 9, 9, 9, 7, 7, 7, 7]) },
    ]);

    // dstStart = ((0 * 4) + 1) * 4 = byte offset 4.
    expect(Array.from(surface.pixels.slice(4, 12))).toEqual([9, 9, 9, 9, 7, 7, 7, 7]);
    expect(Array.from(surface.pixels.slice(0, 4))).toEqual([0, 0, 0, 0]);
    expect(surface.dirty).toEqual({ x: 1, y: 0, width: 2, height: 1 });
  });

  it('copies multi-row rects using the source row stride', () => {
    const surface = makeSurface(4, 2);
    // 2x2 rect at (2,0): source rows are 2 px wide (8 bytes each).
    compositeFrameRects(surface, [
      {
        x: 2,
        y: 0,
        width: 2,
        height: 2,
        pixels: new Uint8Array([
          1, 1, 1, 1, 2, 2, 2, 2,
          3, 3, 3, 3, 4, 4, 4, 4,
        ]),
      },
    ]);

    const px = (x: number, y: number) => Array.from(surface.pixels.slice((y * 4 + x) * 4, (y * 4 + x) * 4 + 4));
    expect(px(2, 0)).toEqual([1, 1, 1, 1]);
    expect(px(3, 0)).toEqual([2, 2, 2, 2]);
    expect(px(2, 1)).toEqual([3, 3, 3, 3]);
    expect(px(3, 1)).toEqual([4, 4, 4, 4]);
    expect(surface.dirty).toEqual({ x: 2, y: 0, width: 2, height: 2 });
  });

  it('clamps rects that extend past the surface bounds', () => {
    const surface = makeSurface(4, 2);
    // Rect (3,1) 4x3 extends past both edges: only 1x1 is in bounds.
    compositeFrameRects(surface, [
      { x: 3, y: 1, width: 4, height: 3, pixels: new Uint8Array(4 * 3 * 4).fill(5) },
    ]);

    expect(Array.from(surface.pixels.slice((1 * 4 + 3) * 4, (1 * 4 + 3) * 4 + 4))).toEqual([5, 5, 5, 5]);
    expect(surface.dirty).toEqual({ x: 3, y: 1, width: 1, height: 1 });
  });

  it('skips rects entirely outside the surface and zero-sized rects', () => {
    const surface = makeSurface(2, 2);
    surface.dirty = { x: 0, y: 0, width: 1, height: 1 };

    compositeFrameRects(surface, [
      { x: 10, y: 0, width: 2, height: 2, pixels: new Uint8Array(16) },
      { x: 0, y: 0, width: 0, height: 5, pixels: new Uint8Array(0) },
      { x: 0, y: 0, width: 5, height: 0, pixels: new Uint8Array(0) },
    ]);

    expect(Array.from(surface.pixels)).toEqual(new Array(16).fill(0));
    expect(surface.dirty).toEqual({ x: 0, y: 0, width: 1, height: 1 });
  });

  it('unions the dirty region across multiple rects and prior state', () => {
    const surface = makeSurface(10, 10);
    surface.dirty = { x: 0, y: 0, width: 1, height: 1 };

    compositeFrameRects(surface, [
      { x: 4, y: 4, width: 1, height: 1, pixels: new Uint8Array(4) },
      { x: 8, y: 8, width: 1, height: 1, pixels: new Uint8Array(4) },
    ]);

    expect(surface.dirty).toEqual({ x: 0, y: 0, width: 9, height: 9 });
  });
});
