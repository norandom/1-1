import { describe, expect, it } from 'vitest';

import { formatBandwidth, fpsLevel, latencyLevel } from '../hud-format';

describe('formatBandwidth', () => {
  it('formats byte rates below 1 KiB', () => {
    expect(formatBandwidth(0)).toBe('0 B/s');
    expect(formatBandwidth(1023)).toBe('1023 B/s');
  });

  it('formats KiB/s and MiB/s at their boundaries', () => {
    expect(formatBandwidth(1024)).toBe('1.0 KiB/s');
    expect(formatBandwidth(1536)).toBe('1.5 KiB/s');
    expect(formatBandwidth(1024 * 1024 - 1)).toBe('1024.0 KiB/s');
    expect(formatBandwidth(1024 * 1024)).toBe('1.0 MiB/s');
    expect(formatBandwidth(3 * 1024 * 1024)).toBe('3.0 MiB/s');
  });
});

describe('fpsLevel', () => {
  it('classifies at the 24 and 15 fps thresholds', () => {
    expect(fpsLevel(24)).toBe('good');
    expect(fpsLevel(60)).toBe('good');
    expect(fpsLevel(23)).toBe('medium');
    expect(fpsLevel(15)).toBe('medium');
    expect(fpsLevel(14)).toBe('bad');
    expect(fpsLevel(0)).toBe('bad');
  });
});

describe('latencyLevel', () => {
  it('classifies at the 50 ms and 150 ms thresholds', () => {
    expect(latencyLevel(0)).toBe('good');
    expect(latencyLevel(49)).toBe('good');
    expect(latencyLevel(50)).toBe('medium');
    expect(latencyLevel(149)).toBe('medium');
    expect(latencyLevel(150)).toBe('bad');
  });
});
