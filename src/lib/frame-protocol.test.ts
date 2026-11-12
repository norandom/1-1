import { describe, expect, it } from 'vitest';

import {
  parseFramePacket,
  tryParseH264Packet,
} from './frame-protocol';
import { isH264Keyframe } from './h264-decoder';

/** Build a wire-format packet the same way the Rust encoder does. */
function buildPacket(
  kind: number,
  surfaceWidth: number,
  surfaceHeight: number,
  rects: Array<{ x: number; y: number; w: number; h: number; pixels: number[] }>
): Uint8Array {
  const header = [
    kind,
    surfaceWidth & 0xff,
    (surfaceWidth >> 8) & 0xff,
    surfaceHeight & 0xff,
    (surfaceHeight >> 8) & 0xff,
    rects.length & 0xff,
    (rects.length >> 8) & 0xff,
  ];
  const rectHeaders = rects.flatMap((r) => [
    r.x & 0xff,
    (r.x >> 8) & 0xff,
    r.y & 0xff,
    (r.y >> 8) & 0xff,
    r.w & 0xff,
    (r.w >> 8) & 0xff,
    r.h & 0xff,
    (r.h >> 8) & 0xff,
  ]);
  const pixels = rects.flatMap((r) => r.pixels);
  return new Uint8Array([...header, ...rectHeaders, ...pixels]);
}

describe('frame protocol', () => {
  it('parses a full-frame packet', () => {
    const packet = new Uint8Array([
      1,
      2, 0,
      1, 0,
      1, 0,
      0, 0,
      0, 0,
      2, 0,
      1, 0,
      1, 2, 3, 4, 5, 6, 7, 8,
    ]);

    const parsed = parseFramePacket(packet);

    expect(parsed.kind).toBe('full-frame');
    expect(parsed.surfaceWidth).toBe(2);
    expect(parsed.surfaceHeight).toBe(1);
    expect(parsed.rects).toHaveLength(1);
    expect(parsed.rects[0]).toMatchObject({
      x: 0,
      y: 0,
      width: 2,
      height: 1,
    });
    expect(Array.from(parsed.rects[0].pixels)).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
  });

  it('parses a dirty-rects packet with multiple rects at offsets', () => {
    const packet = buildPacket(2, 4, 4, [
      { x: 1, y: 2, w: 2, h: 1, pixels: [1, 1, 1, 1, 2, 2, 2, 2] },
      { x: 0, y: 0, w: 1, h: 2, pixels: [9, 9, 9, 9, 8, 8, 8, 8] },
    ]);

    const parsed = parseFramePacket(packet);

    expect(parsed.kind).toBe('dirty-rects');
    expect(parsed.surfaceWidth).toBe(4);
    expect(parsed.surfaceHeight).toBe(4);
    expect(parsed.rects).toHaveLength(2);
    // Rect payloads are concatenated in rect order after the rect headers.
    expect(parsed.rects[0]).toMatchObject({ x: 1, y: 2, width: 2, height: 1 });
    expect(Array.from(parsed.rects[0].pixels)).toEqual([1, 1, 1, 1, 2, 2, 2, 2]);
    expect(parsed.rects[1]).toMatchObject({ x: 0, y: 0, width: 1, height: 2 });
    expect(Array.from(parsed.rects[1].pixels)).toEqual([9, 9, 9, 9, 8, 8, 8, 8]);
  });

  it('accepts an ArrayBuffer payload (channel delivery shape)', () => {
    const bytes = buildPacket(1, 1, 1, [
      { x: 0, y: 0, w: 1, h: 1, pixels: [7, 7, 7, 7] },
    ]);
    const buffer = bytes.buffer.slice(
      bytes.byteOffset,
      bytes.byteOffset + bytes.byteLength
    ) as ArrayBuffer;

    const parsed = parseFramePacket(buffer);

    expect(parsed.kind).toBe('full-frame');
    expect(Array.from(parsed.rects[0].pixels)).toEqual([7, 7, 7, 7]);
  });

  it('accepts zero-rect packets with only a header', () => {
    const parsed = parseFramePacket(buildPacket(1, 4, 2, []));

    expect(parsed.kind).toBe('full-frame');
    expect(parsed.rects).toEqual([]);
  });

  it('rejects payloads shorter than the packet header', () => {
    expect(() => parseFramePacket(new Uint8Array([1, 2, 0, 1, 0, 1]))).toThrow(
      'Frame packet too short'
    );
  });

  it('rejects truncated rect metadata', () => {
    // Header claims 2 rects but only one rect header is present.
    const packet = new Uint8Array([2, 4, 0, 4, 0, 2, 0, 0, 0, 1, 0, 1, 0]);
    expect(() => parseFramePacket(packet)).toThrow(
      'Frame packet metadata is truncated'
    );
  });

  it('rejects truncated pixel payloads', () => {
    const packet = buildPacket(2, 2, 1, [
      { x: 0, y: 0, w: 2, h: 1, pixels: [1, 2, 3, 4, 5, 6] },
    ]);
    expect(() => parseFramePacket(packet)).toThrow(
      'Frame packet pixel payload is truncated'
    );
  });

  it('rejects trailing bytes after the last rect payload', () => {
    const packet = buildPacket(1, 1, 1, [
      { x: 0, y: 0, w: 1, h: 1, pixels: [1, 2, 3, 4] },
    ]);
    const withTrailing = new Uint8Array([...packet, 0xff]);
    expect(() => parseFramePacket(withTrailing)).toThrow(
      'Frame packet has trailing bytes'
    );
  });

  it('rejects unknown packet kinds', () => {
    expect(() => parseFramePacket(buildPacket(0, 1, 1, []))).toThrow(
      'Unknown frame packet kind: 0'
    );
    // Kind 3 is the H.264 packet and must go through tryParseH264Packet.
    expect(() => parseFramePacket(buildPacket(3, 1, 1, []))).toThrow(
      'Unknown frame packet kind: 3'
    );
  });
});

describe('tryParseH264Packet', () => {
  it('parses a kind-3 packet with its documented layout', () => {
    const packet = new Uint8Array([
      3,
      2, 0, // width = 2 (LE)
      1, 0, // height = 1 (LE)
      4, 0, 0, 0, // dataLen = 4 (LE)
      0x11, 0x22, 0x33, 0x44,
    ]);

    const parsed = tryParseH264Packet(packet);

    expect(parsed).not.toBeNull();
    expect(parsed?.kind).toBe('h264');
    expect(parsed?.surfaceWidth).toBe(2);
    expect(parsed?.surfaceHeight).toBe(1);
    expect(Array.from(parsed?.h264Data ?? [])).toEqual([0x11, 0x22, 0x33, 0x44]);
  });

  it('returns null for non-H.264 and too-short payloads', () => {
    const fullFrame = buildPacket(1, 2, 1, [
      { x: 0, y: 0, w: 2, h: 1, pixels: [1, 2, 3, 4, 5, 6, 7, 8] },
    ]);
    expect(tryParseH264Packet(fullFrame)).toBeNull();
    expect(tryParseH264Packet(new Uint8Array([3, 2, 0, 1, 0, 4, 0, 0]))).toBeNull();
  });

  it('clamps the declared data length to the available bytes (no validation)', () => {
    // dataLen claims 16 but only 4 bytes follow; the parser silently clamps.
    const packet = new Uint8Array([
      3, 2, 0, 1, 0, 16, 0, 0, 0, 1, 2, 3, 4,
    ]);

    const parsed = tryParseH264Packet(packet);

    expect(Array.from(parsed?.h264Data ?? [])).toEqual([1, 2, 3, 4]);
  });
});

describe('isH264Keyframe', () => {
  it('detects SPS and IDR NAL units after Annex B start codes', () => {
    expect(isH264Keyframe(new Uint8Array([0x00, 0x00, 0x01, 0x67, 0x00]))).toBe(true);
    expect(isH264Keyframe(new Uint8Array([0x00, 0x00, 0x01, 0x65, 0x00]))).toBe(true);
    expect(isH264Keyframe(new Uint8Array([0x00, 0x00, 0x00, 0x01, 0x65]))).toBe(true);
  });

  it('returns false for non-keyframe slices and start-code-free data', () => {
    expect(isH264Keyframe(new Uint8Array([0x00, 0x00, 0x01, 0x41, 0x00]))).toBe(false);
    expect(isH264Keyframe(new Uint8Array([0x67, 0x00, 0x00, 0x01, 0x65]))).toBe(false);
  });
});
