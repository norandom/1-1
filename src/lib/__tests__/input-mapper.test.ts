import { describe, it, expect } from 'vitest';
import {
  mapKeyEvent,
  mapMouseEvent,
  normalizeWheelDelta,
  ctrlAltDelSequence,
} from '../input-mapper';

const mockKeyEvent = (overrides: Partial<KeyboardEvent>) =>
  ({
    code: 'KeyA',
    key: 'a',
    type: 'keydown',
    ...overrides,
  }) as unknown as KeyboardEvent;

const mockMouseEvent = (overrides: Partial<MouseEvent>) =>
  ({
    clientX: 100,
    clientY: 200,
    button: 0,
    ...overrides,
  }) as unknown as MouseEvent;

describe('mapKeyEvent', () => {
  it('maps letter keys correctly', () => {
    const result = mapKeyEvent(mockKeyEvent({ code: 'KeyA', type: 'keydown' }));
    expect(result.keyCode).toBe(0x1e);
    expect(result.isDown).toBe(true);
  });

  it('maps keyup events', () => {
    const result = mapKeyEvent(mockKeyEvent({ code: 'KeyA', type: 'keyup' }));
    expect(result.keyCode).toBe(0x1e);
    expect(result.isDown).toBe(false);
  });

  it('maps Enter key', () => {
    const result = mapKeyEvent(mockKeyEvent({ code: 'Enter', key: 'Enter' }));
    expect(result.keyCode).toBe(0x1c);
  });

  it('maps F1 key', () => {
    const result = mapKeyEvent(mockKeyEvent({ code: 'F1', key: 'F1' }));
    expect(result.keyCode).toBe(0x3b);
  });

  it('maps ShiftLeft modifier', () => {
    const result = mapKeyEvent(mockKeyEvent({ code: 'ShiftLeft', key: 'Shift' }));
    expect(result.keyCode).toBe(0x2a);
  });

  it('maps ControlLeft modifier', () => {
    const result = mapKeyEvent(mockKeyEvent({ code: 'ControlLeft', key: 'Control' }));
    expect(result.keyCode).toBe(0x1d);
  });

  it('maps AltLeft modifier', () => {
    const result = mapKeyEvent(mockKeyEvent({ code: 'AltLeft', key: 'Alt' }));
    expect(result.keyCode).toBe(0x38);
  });

  it('marks extended keys in the encoded keyCode', () => {
    const result = mapKeyEvent(mockKeyEvent({ code: 'Delete', key: 'Delete' }));
    expect(result.keyCode).toBe(0x153);
  });

  it('maps right-hand modifiers with the extended flag (Req 3.1)', () => {
    expect(mapKeyEvent(mockKeyEvent({ code: 'ShiftRight' })).keyCode).toBe(0x36);
    // ControlRight/AltRight share the left scancode plus 0x100.
    expect(mapKeyEvent(mockKeyEvent({ code: 'ControlRight' })).keyCode).toBe(0x11d);
    expect(mapKeyEvent(mockKeyEvent({ code: 'AltRight' })).keyCode).toBe(0x138);
  });

  it('maps OS meta keys as extended keys (Req 3.1)', () => {
    expect(mapKeyEvent(mockKeyEvent({ code: 'MetaLeft' })).keyCode).toBe(0x15b);
    expect(mapKeyEvent(mockKeyEvent({ code: 'MetaRight' })).keyCode).toBe(0x15c);
  });

  it('maps NumpadEnter as the extended Enter variant', () => {
    expect(mapKeyEvent(mockKeyEvent({ code: 'NumpadEnter' })).keyCode).toBe(0x11c);
    expect(mapKeyEvent(mockKeyEvent({ code: 'Enter' })).keyCode).toBe(0x1c);
  });

  it('returns keyCode 0 for unmapped keys (dropped by the sender and the actor)', () => {
    const result = mapKeyEvent(
      mockKeyEvent({ code: 'MediaPlayPause', type: 'keydown' })
    );
    expect(result.keyCode).toBe(0);
    expect(result.isDown).toBe(true);
    const up = mapKeyEvent(mockKeyEvent({ code: 'MediaPlayPause', type: 'keyup' }));
    expect(up.keyCode).toBe(0);
    expect(up.isDown).toBe(false);
  });

  it('reports modifier key-up with isDown false', () => {
    const result = mapKeyEvent(
      mockKeyEvent({ code: 'ControlLeft', type: 'keyup' })
    );
    expect(result.keyCode).toBe(0x1d);
    expect(result.isDown).toBe(false);
  });
});

describe('mapMouseEvent', () => {
  it('maps left click (button 0 -> 1)', () => {
    const result = mapMouseEvent(mockMouseEvent({ button: 0 }), 'down');
    expect(result.button).toBe(1);
  });

  it('maps right click (button 2 -> 2)', () => {
    const result = mapMouseEvent(mockMouseEvent({ button: 2 }), 'down');
    expect(result.button).toBe(2);
  });

  it('maps middle click (button 1 -> 3)', () => {
    const result = mapMouseEvent(mockMouseEvent({ button: 1 }), 'down');
    expect(result.button).toBe(3);
  });

  it('computes coordinates relative to canvas rect', () => {
    const rect = { left: 10, top: 20 } as DOMRect;
    const result = mapMouseEvent(
      mockMouseEvent({ clientX: 100, clientY: 200 }),
      'move',
      rect
    );
    expect(result.x).toBe(90);
    expect(result.y).toBe(180);
  });

  it('scales coordinates to the remote surface size', () => {
    const result = mapMouseEvent(
      mockMouseEvent({ clientX: 110, clientY: 70 }),
      'move',
      {
        rect: { left: 10, top: 20, width: 200, height: 100 } as DOMRect,
        surfaceWidth: 1920,
        surfaceHeight: 1080,
      }
    );

    expect(result.x).toBe(960);
    expect(result.y).toBe(540);
  });

  it('uses clientX/clientY without rect', () => {
    const result = mapMouseEvent(
      mockMouseEvent({ clientX: 100, clientY: 200 }),
      'move'
    );
    expect(result.x).toBe(100);
    expect(result.y).toBe(200);
  });

  it('sets the correct eventType', () => {
    const result = mapMouseEvent(mockMouseEvent({}), 'scroll');
    expect(result.eventType).toBe('scroll');
  });

  it('clamps coordinates to the remote surface on both axes (Req 3.2)', () => {
    const bounds = {
      rect: { left: 0, top: 0, width: 200, height: 100 } as DOMRect,
      surfaceWidth: 1920,
      surfaceHeight: 1080,
    };
    // Pointer beyond the right/bottom edge of the visible rect.
    const beyond = mapMouseEvent(mockMouseEvent({ clientX: 250, clientY: 150 }), 'move', bounds);
    expect(beyond.x).toBe(1919);
    expect(beyond.y).toBe(1079);
    // Pointer above/left of the rect (negative relative coordinates).
    const negative = mapMouseEvent(mockMouseEvent({ clientX: -30, clientY: -10 }), 'move', bounds);
    expect(negative.x).toBe(0);
    expect(negative.y).toBe(0);
  });

  it('rounds half-up when scaling fractional pixel positions', () => {
    const result = mapMouseEvent(
      mockMouseEvent({ clientX: 10, clientY: 10 }),
      'move',
      {
        rect: { left: 0, top: 0, width: 200, height: 100 } as DOMRect,
        surfaceWidth: 1921,
        surfaceHeight: 1081,
      }
    );
    // relativeX 10 * (1921/200) = 96.05 -> 96; relativeY 10 * (1081/100) = 108.1 -> 108
    expect(result.x).toBe(96);
    expect(result.y).toBe(108);
  });

  it('guards a degenerate (zero-sized) rect against division by zero', () => {
    const result = mapMouseEvent(
      mockMouseEvent({ clientX: 50, clientY: 50 }),
      'move',
      {
        rect: { left: 0, top: 0, width: 0, height: 0 } as DOMRect,
        surfaceWidth: 1920,
        surfaceHeight: 1080,
      }
    );
    // scaleX = 1920 / max(0, 1) -> relativeX 50 * 1920 clamped to 1919.
    expect(result.x).toBe(1919);
    expect(result.y).toBe(1079);
  });
});

describe('normalizeWheelDelta (Req 3.2)', () => {
  it('converts pixel-mode deltaY into signed rotation units', () => {
    // Browser scroll down (positive deltaY) becomes negative (down) units.
    expect(normalizeWheelDelta(100, 0)).toBe(-3); // 100/40 = 2.5 rounds to 3
    expect(normalizeWheelDelta(-100, 0)).toBe(3);
  });

  it('always emits at least one unit in the scrolled direction', () => {
    // Any nonzero pixel delta maps to at least one unit…
    expect(normalizeWheelDelta(20, 0)).toBe(-1);
    expect(normalizeWheelDelta(0.5, 0)).toBe(-1);
    expect(normalizeWheelDelta(-0.5, 0)).toBe(1);
    // …and a zero delta stays zero — pinned as negative zero, which the
    // sign flip produces and JSON serialization sends as plain 0.
    expect(Object.is(normalizeWheelDelta(0, 0), -0)).toBe(true);
  });

  it('multiplies line-mode deltas by three units per line', () => {
    expect(normalizeWheelDelta(3, 1)).toBe(-9);
    expect(normalizeWheelDelta(-3, 1)).toBe(9);
    // Zero in line mode is also negative zero after the sign flip.
    expect(Object.is(normalizeWheelDelta(0, 1), -0)).toBe(true);
  });

  it('clamps to the ±255 RDP wheel-unit maximum', () => {
    expect(normalizeWheelDelta(20000, 0)).toBe(-255);
    expect(normalizeWheelDelta(-20000, 0)).toBe(255);
    expect(normalizeWheelDelta(100, 1)).toBe(-255); // 100 lines * 3 = 300
  });
});

describe('ctrlAltDelSequence (Req 3.4)', () => {
  it('sends Ctrl and Alt down, taps extended Delete, releases in reverse', () => {
    expect(ctrlAltDelSequence()).toEqual([
      { keyCode: 0x1d, isDown: true }, // Left Ctrl
      { keyCode: 0x38, isDown: true }, // Left Alt
      { keyCode: 0x153, isDown: true }, // Extended Delete
      { keyCode: 0x153, isDown: false },
      { keyCode: 0x38, isDown: false },
      { keyCode: 0x1d, isDown: false },
    ]);
  });

  it('returns a fresh array per call', () => {
    expect(ctrlAltDelSequence()).not.toBe(ctrlAltDelSequence());
  });
});
