import { describe, expect, it } from 'vitest';
import { desktopScale, physicalViewport, remoteViewport, sessionPixelRatio } from '../viewport-size';

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

describe('display scaling', () => {
  it('requests this display density as the desktop scale in Automatic', () => {
    // A Retina MacBook window: 1440 x 900 CSS pixels at 2x.
    expect(remoteViewport(1440, 900, 'automatic', 2)).toEqual({ width: 2880, height: 1800, scale: 200 });
    expect(remoteViewport(1280, 720, 'automatic', 1.25)).toEqual({ width: 1600, height: 900, scale: 125 });
    expect(remoteViewport(1280, 720, 'automatic', 1)).toEqual({ width: 1280, height: 720, scale: 100 });
  });
  it('keeps physical pixels at 100% when scaling is off', () => {
    expect(remoteViewport(1440, 900, 'off', 2)).toEqual({ width: 2880, height: 1800, scale: 100 });
  });
  it('sends one remote pixel per CSS pixel at 100% in low resolution', () => {
    expect(remoteViewport(1440, 900, 'lowResolution', 2)).toEqual({ width: 1440, height: 900, scale: 100 });
    expect(sessionPixelRatio('lowResolution', 2)).toBe(1);
    expect(sessionPixelRatio('automatic', 2)).toBe(2);
  });
  it('uses the chosen percentage for Custom, whatever the display density', () => {
    expect(remoteViewport(1440, 900, 'custom', 2, 150)).toEqual({ width: 2880, height: 1800, scale: 150 });
    expect(remoteViewport(1280, 720, 'custom', 1, 125)).toEqual({ width: 1280, height: 720, scale: 125 });
    expect(desktopScale('custom', 2, 900)).toBe(500);
    expect(desktopScale('custom', 2, NaN)).toBe(100);
    expect(desktopScale('off', 2, 200)).toBe(100);
  });
  it('rounds fractional densities and stays within RDP limits', () => {
    expect(desktopScale('automatic', 5 / 3)).toBe(167);
    expect(desktopScale('automatic', 0.8)).toBe(100);
    expect(desktopScale('automatic', 6)).toBe(500);
    expect(desktopScale('automatic', NaN)).toBe(100);
  });
});