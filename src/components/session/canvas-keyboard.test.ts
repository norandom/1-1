import { afterEach, expect, it, vi } from 'vitest';
import { matchKeyboardShortcut } from '../../lib/shortcuts';
import { subscribeCanvasKeyboard } from './canvas-keyboard';

class Key extends Event {
  code = '';
  key = '';
  ctrlKey = false;
  altKey = false;
  shiftKey = false;
  metaKey = false;
  constructor(type: string, init: Partial<KeyboardEvent> = {}) {
    super(type, { cancelable: true });
    Object.assign(this, init);
  }
}
afterEach(() => vi.unstubAllGlobals());

it.each([true, false])('focuses display size without toggling HUD regardless of listener order (%s)', (globalFirst) => {
  vi.stubGlobal('KeyboardEvent', Key);
  const target = new EventTarget();
  const select = Object.assign(new EventTarget(), { focus: vi.fn() });
  const down = vi.fn();
  const up = vi.fn();
  const actions: string[] = [];
  const globalListener = (event: Event) => {
    const action = matchKeyboardShortcut(event as KeyboardEvent);
    if (action) actions.push(action);
  };
  if (globalFirst) target.addEventListener('keydown', globalListener);
  const cleanup = subscribeCanvasKeyboard(target, select, down, up);
  if (!globalFirst) target.addEventListener('keydown', globalListener);
  for (const code of ['ControlLeft', 'AltLeft', 'ShiftLeft']) target.dispatchEvent(new Key('keydown', { code }));
  const chord = new Key('keydown', { code: 'KeyP', key: 'P', ctrlKey: true, altKey: true, shiftKey: true });
  target.dispatchEvent(chord);
  expect(chord.defaultPrevented).toBe(true);
  expect(select.focus).toHaveBeenCalledOnce();
  expect(down).toHaveBeenCalledTimes(3);
  expect(up.mock.calls.map(([event]) => event.code)).toEqual(['ControlLeft', 'AltLeft', 'ShiftLeft']);
  expect(actions).toEqual([]);
  target.dispatchEvent(new Key('keydown', { code: 'KeyP', key: 'P', ctrlKey: true, shiftKey: true }));
  expect(actions).toEqual(['performance-hud']);
  cleanup();
});

it('preserves remote Tab, releases held keys on control focus, and removes both subscriptions', () => {
  vi.stubGlobal('KeyboardEvent', Key);
  const target = new EventTarget();
  const select = Object.assign(new EventTarget(), { focus: vi.fn() });
  const down = vi.fn();
  const up = vi.fn();
  const cleanup = subscribeCanvasKeyboard(target, select, down, up);
  const tab = new Key('keydown', { code: 'Tab', key: 'Tab' });
  target.dispatchEvent(tab);
  expect(tab.defaultPrevented).toBe(true);
  expect(down).toHaveBeenCalledExactlyOnceWith(tab);
  expect(select.focus).not.toHaveBeenCalled();
  select.dispatchEvent(new Event('focus'));
  expect(up.mock.calls.map(([event]) => event.code)).toEqual(['Tab']);
  select.dispatchEvent(new Event('focus'));
  expect(up).toHaveBeenCalledTimes(1);
  target.dispatchEvent(new Key('keydown', { code: 'ControlLeft' }));
  cleanup();
  expect(up.mock.calls.map(([event]) => event.code)).toEqual(['Tab', 'ControlLeft']);
  const after = new Key('keydown', { code: 'Tab' });
  target.dispatchEvent(after);
  select.dispatchEvent(new Event('focus'));
  expect(after.defaultPrevented).toBe(false);
  expect(down).toHaveBeenCalledTimes(2);
  expect(up).toHaveBeenCalledTimes(2);
});

it('leaves select arrows and Return to native mode selection without remote forwarding', () => {
  vi.stubGlobal('KeyboardEvent', Key);
  // The browser bubbles these keys with the SELECT as event.target.
  const target = Object.assign(new EventTarget(), { tagName: 'SELECT' });
  const select = Object.assign(new EventTarget(), { focus: vi.fn() });
  const down = vi.fn();
  const up = vi.fn();
  const cleanup = subscribeCanvasKeyboard(target, select, down, up);
  for (const code of ['ArrowDown', 'Enter']) {
    for (const type of ['keydown', 'keyup']) {
      const event = new Key(type, { code, key: code });
      target.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
    }
  }
  expect(down).not.toHaveBeenCalled();
  expect(up).not.toHaveBeenCalled();
  cleanup();
});

it.each(['BUTTON', 'A', 'SUMMARY'])('keeps local %s keys out of the remote session', (tagName) => {
  vi.stubGlobal('KeyboardEvent', Key);
  const target = Object.assign(new EventTarget(), { tagName });
  const down = vi.fn();
  const up = vi.fn();
  const cleanup = subscribeCanvasKeyboard(target, null, down, up);
  target.dispatchEvent(new Key('keydown', { code: 'Enter' }));
  target.dispatchEvent(new Key('keyup', { code: 'Enter' }));
  expect(down).not.toHaveBeenCalled();
  expect(up).not.toHaveBeenCalled();
  cleanup();
});

it('releases remote modifiers on blur and local pointer entry before focus changes', () => {
  vi.stubGlobal('KeyboardEvent', Key);
  const target = new EventTarget();
  const down = vi.fn();
  const up = vi.fn();
  const cleanup = subscribeCanvasKeyboard(target, null, down, up);
  target.dispatchEvent(new Key('keydown', { code: 'ControlLeft' }));
  target.dispatchEvent(new Event('blur'));
  expect(up.mock.calls.map(([e]) => e.code)).toEqual(['ControlLeft']);
  target.dispatchEvent(new Key('keydown', { code: 'AltLeft' }));
  Object.assign(target, { tagName: 'BUTTON' });
  target.dispatchEvent(new Event('pointerdown'));
  expect(up.mock.calls.map(([e]) => e.code)).toEqual(['ControlLeft', 'AltLeft']);
  cleanup();
});

it.each(['menu', 'dialog', 'tablist'])('isolates portalled/local %s descendants and releases previously forwarded keys', (role) => {
  vi.stubGlobal('KeyboardEvent', Key);
  const target = Object.assign(new EventTarget(), { closest: vi.fn().mockReturnValue(null) });
  const down = vi.fn();
  const up = vi.fn();
  const cleanup = subscribeCanvasKeyboard(target, null, down, up);
  target.dispatchEvent(new Key('keydown', { code: 'ShiftLeft' }));
  target.closest.mockReturnValue({ role });
  target.dispatchEvent(new Event('focusin'));
  for (const type of ['keydown', 'keyup']) {
    const event = new Key(type, { code: 'ArrowDown' });
    target.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  }
  expect(down).toHaveBeenCalledTimes(1);
  expect(up.mock.calls.map(([e]) => e.code)).toEqual(['ShiftLeft']);
  cleanup();
});
