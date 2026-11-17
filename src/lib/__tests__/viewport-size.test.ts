import { describe, expect, it } from 'vitest';
import { physicalViewport } from '../viewport-size';

describe('physical viewport requests', () => {
  it.each([1, 1.25, 1.5, 2])('converts CSS dimensions exactly once at DPR %s', (dpr) => {
    expect(physicalViewport(1000, 600, dpr)).toEqual({ width: 1000 * dpr, height: 600 * dpr });
  });
  it('normalizes even width and protocol bounds', () => {
    expect(physicalViewport(1001, 601, 1)).toEqual({ width: 1000, height: 601 });
    expect(physicalViewport(20, 30, 1)).toEqual({ width: 200, height: 200 });
    expect(physicalViewport(9000, 9000, 2)).toEqual({ width: 8192, height: 8192 });
  });
  it.each([[0, 600, 1], [1000, -1, 1], [1000, 600, 0], [NaN, 600, 1], [1000, Infinity, 1]])('does not invent a size for unavailable/invalid layout (%s,%s,%s)', (w, h, dpr) => {
    expect(physicalViewport(w, h, dpr)).toBeUndefined();
  });
});
