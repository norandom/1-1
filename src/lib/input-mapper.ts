import type { KeyEvent, MouseEvent as RdpMouseEvent } from '../types';

export interface MouseSurfaceBounds {
  rect: DOMRect;
  surfaceWidth: number;
  surfaceHeight: number;
}

interface ScancodeMapping {
  scancode: number;
  extended?: boolean;
}

const EXTENDED_KEY_FLAG = 0x100;

const SCANCODE_MAP: Record<string, ScancodeMapping> = {
  KeyA: { scancode: 0x1e },
  KeyB: { scancode: 0x30 },
  KeyC: { scancode: 0x2e },
  KeyD: { scancode: 0x20 },
  KeyE: { scancode: 0x12 },
  KeyF: { scancode: 0x21 },
  KeyG: { scancode: 0x22 },
  KeyH: { scancode: 0x23 },
  KeyI: { scancode: 0x17 },
  KeyJ: { scancode: 0x24 },
  KeyK: { scancode: 0x25 },
  KeyL: { scancode: 0x26 },
  KeyM: { scancode: 0x32 },
  KeyN: { scancode: 0x31 },
  KeyO: { scancode: 0x18 },
  KeyP: { scancode: 0x19 },
  KeyQ: { scancode: 0x10 },
  KeyR: { scancode: 0x13 },
  KeyS: { scancode: 0x1f },
  KeyT: { scancode: 0x14 },
  KeyU: { scancode: 0x16 },
  KeyV: { scancode: 0x2f },
  KeyW: { scancode: 0x11 },
  KeyX: { scancode: 0x2d },
  KeyY: { scancode: 0x15 },
  KeyZ: { scancode: 0x2c },
  Digit1: { scancode: 0x02 },
  Digit2: { scancode: 0x03 },
  Digit3: { scancode: 0x04 },
  Digit4: { scancode: 0x05 },
  Digit5: { scancode: 0x06 },
  Digit6: { scancode: 0x07 },
  Digit7: { scancode: 0x08 },
  Digit8: { scancode: 0x09 },
  Digit9: { scancode: 0x0a },
  Digit0: { scancode: 0x0b },
  Escape: { scancode: 0x01 },
  Backspace: { scancode: 0x0e },
  Tab: { scancode: 0x0f },
  Enter: { scancode: 0x1c },
  NumpadEnter: { scancode: 0x1c, extended: true },
  Space: { scancode: 0x39 },
  Minus: { scancode: 0x0c },
  Equal: { scancode: 0x0d },
  BracketLeft: { scancode: 0x1a },
  BracketRight: { scancode: 0x1b },
  Backslash: { scancode: 0x2b },
  IntlBackslash: { scancode: 0x56 },
  Semicolon: { scancode: 0x27 },
  Quote: { scancode: 0x28 },
  Backquote: { scancode: 0x29 },
  Comma: { scancode: 0x33 },
  Period: { scancode: 0x34 },
  Slash: { scancode: 0x35 },
  CapsLock: { scancode: 0x3a },
  F1: { scancode: 0x3b },
  F2: { scancode: 0x3c },
  F3: { scancode: 0x3d },
  F4: { scancode: 0x3e },
  F5: { scancode: 0x3f },
  F6: { scancode: 0x40 },
  F7: { scancode: 0x41 },
  F8: { scancode: 0x42 },
  F9: { scancode: 0x43 },
  F10: { scancode: 0x44 },
  F11: { scancode: 0x57 },
  F12: { scancode: 0x58 },
  PrintScreen: { scancode: 0x37, extended: true },
  ScrollLock: { scancode: 0x46 },
  Pause: { scancode: 0x45 },
  Insert: { scancode: 0x52, extended: true },
  Delete: { scancode: 0x53, extended: true },
  Home: { scancode: 0x47, extended: true },
  End: { scancode: 0x4f, extended: true },
  PageUp: { scancode: 0x49, extended: true },
  PageDown: { scancode: 0x51, extended: true },
  ArrowUp: { scancode: 0x48, extended: true },
  ArrowDown: { scancode: 0x50, extended: true },
  ArrowLeft: { scancode: 0x4b, extended: true },
  ArrowRight: { scancode: 0x4d, extended: true },
  NumLock: { scancode: 0x45 },
  NumpadDivide: { scancode: 0x35, extended: true },
  NumpadMultiply: { scancode: 0x37 },
  NumpadSubtract: { scancode: 0x4a },
  NumpadAdd: { scancode: 0x4e },
  NumpadDecimal: { scancode: 0x53 },
  Numpad0: { scancode: 0x52 },
  Numpad1: { scancode: 0x4f },
  Numpad2: { scancode: 0x50 },
  Numpad3: { scancode: 0x51 },
  Numpad4: { scancode: 0x4b },
  Numpad5: { scancode: 0x4c },
  Numpad6: { scancode: 0x4d },
  Numpad7: { scancode: 0x47 },
  Numpad8: { scancode: 0x48 },
  Numpad9: { scancode: 0x49 },
  ShiftLeft: { scancode: 0x2a },
  ShiftRight: { scancode: 0x36 },
  ControlLeft: { scancode: 0x1d },
  ControlRight: { scancode: 0x1d, extended: true },
  AltLeft: { scancode: 0x38 },
  AltRight: { scancode: 0x38, extended: true },
  MetaLeft: { scancode: 0x5b, extended: true },
  MetaRight: { scancode: 0x5c, extended: true },
  ContextMenu: { scancode: 0x5d, extended: true },
};

export function mapKeyEvent(e: globalThis.KeyboardEvent): KeyEvent {
  const mapping = SCANCODE_MAP[e.code] ?? null;
  const keyCode =
    mapping === null
      ? 0
      : mapping.scancode | (mapping.extended ? EXTENDED_KEY_FLAG : 0);

  return {
    keyCode,
    isDown: e.type === 'keydown',
  };
}

export function mapMouseEvent(
  e: globalThis.MouseEvent,
  type: 'move' | 'down' | 'up' | 'scroll',
  bounds?: MouseSurfaceBounds | DOMRect
): RdpMouseEvent {
  if (!bounds) {
    return {
      x: Math.round(e.clientX),
      y: Math.round(e.clientY),
      button: mapMouseButton(e.button),
      eventType: type,
    };
  }

  if (!('rect' in bounds)) {
    return {
      x: Math.round(e.clientX - bounds.left),
      y: Math.round(e.clientY - bounds.top),
      button: mapMouseButton(e.button),
      eventType: type,
    };
  }

  const relativeX = e.clientX - bounds.rect.left;
  const relativeY = e.clientY - bounds.rect.top;
  const scaleX = bounds.surfaceWidth / Math.max(bounds.rect.width, 1);
  const scaleY = bounds.surfaceHeight / Math.max(bounds.rect.height, 1);
  const maxX = Math.max(bounds.surfaceWidth - 1, 0);
  const maxY = Math.max(bounds.surfaceHeight - 1, 0);

  return {
    x: Math.max(0, Math.min(maxX, Math.round(relativeX * scaleX))),
    y: Math.max(0, Math.min(maxY, Math.round(relativeY * scaleY))),
    button: mapMouseButton(e.button),
    eventType: type,
  };
}

function mapMouseButton(button: number): number {
  switch (button) {
    case 0:
      return 1;
    case 1:
      return 3;
    case 2:
      return 2;
    default:
      return 0;
  }
}

/**
 * Convert a browser wheel `deltaY` (with its `deltaMode`) into RDP wheel
 * rotation units. Browser deltaY is positive for scrolling down; RDP wheel
 * units are positive for scrolling up, so the sign is flipped. Line mode
 * (deltaMode 1) multiplies by 3 units per line; pixel mode (and any other
 * mode) normalizes ~40px to one unit, always at least one unit in the
 * scrolled direction, clamped to the RDP maximum of 255 units.
 */
export function normalizeWheelDelta(deltaY: number, deltaMode: number): number {
  let delta = -deltaY;
  if (deltaMode === 1) {
    // Line mode (mouse wheel) — each line = 3 rotation units
    delta *= 3;
  } else {
    // Pixel mode (trackpad) — normalize: ~40px = 1 rotation unit
    delta = Math.sign(delta) * Math.max(1, Math.round(Math.abs(delta) / 40));
  }

  // Clamp to RDP max (255 rotation units)
  return Math.max(-255, Math.min(255, delta));
}

/**
 * Key event sequence for the Ctrl+Alt+Del special-key action: left Ctrl
 * (0x1d) and left Alt (0x38) held down while extended Delete (0x53 with the
 * 0x100 extended flag) is tapped, then released in reverse order.
 */
export function ctrlAltDelSequence(): KeyEvent[] {
  return [
    { keyCode: 0x1d, isDown: true },
    { keyCode: 0x38, isDown: true },
    { keyCode: 0x153, isDown: true },
    { keyCode: 0x153, isDown: false },
    { keyCode: 0x38, isDown: false },
    { keyCode: 0x1d, isDown: false },
  ];
}
