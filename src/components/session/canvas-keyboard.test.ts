import { afterEach, expect, it, vi } from 'vitest';
import { matchKeyboardShortcut } from '../../lib/shortcuts';
import { subscribeClipboardInput } from '../../lib/clipboard-input';
import { subscribeCanvasKeyboard, subscribeSessionEscape } from './canvas-keyboard';

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

function pausedCanvas() {
  vi.stubGlobal('KeyboardEvent', Key);
  const canvas = Object.assign(new EventTarget(), { tagName: 'CANVAS' });
  const menu = Object.assign(new EventTarget(), { focus: vi.fn() });
  const bar = Object.assign(new EventTarget(), { focus: vi.fn() });
  const sent: string[] = [];
  const stop = subscribeCanvasKeyboard(canvas, menu,
    (e) => sent.push('down ' + e.code), (e) => sent.push('up ' + e.code), bar,
    { isPaused: () => true, canvas });
  const key = (type: 'keydown' | 'keyup', code: string, held: Partial<KeyboardEvent> = {}) =>
    canvas.dispatchEvent(new Key(type, { code, ...held }));
  return { canvas, menu, bar, sent, stop, key };
}

it.each(['KeyS', 'KeyP'])('keeps a paused session asleep for the entire physical %s local chord', (code) => {
  const { bar, menu, sent, key, stop } = pausedCanvas();
  key('keydown', 'ControlRight', { ctrlKey: true });
  key('keydown', 'ShiftLeft', { ctrlKey: true, shiftKey: true });
  key('keydown', 'AltLeft', { ctrlKey: true, shiftKey: true, altKey: true });
  key('keydown', code, { ctrlKey: true, shiftKey: true, altKey: true });
  for (const physical of ['ControlRight', 'ShiftLeft', 'AltLeft', code]) key('keyup', physical);
  expect(code === 'KeyS' ? bar.focus : menu.focus).toHaveBeenCalledOnce();
  expect(sent).toEqual([]);
  stop();
});

it.each(['mousedown', 'wheel', 'keydown'])('flushes physical modifiers in order before a remote %s', (type) => {
  const { canvas, sent, key, stop } = pausedCanvas();
  key('keydown', 'ShiftRight', { shiftKey: true });
  key('keydown', 'ControlLeft', { shiftKey: true, ctrlKey: true });
  canvas.dispatchEvent(new Event('pointermove'));
  expect(sent).toEqual([]);
  const held = { shiftKey: true, ctrlKey: true };
  if (type === 'keydown') key('keydown', 'KeyC', held);
  else {
    canvas.addEventListener(type, () => sent.push(type));
    canvas.dispatchEvent(Object.assign(new Event(type), held));
  }
  expect(sent).toEqual(['down ShiftRight', 'down ControlLeft', type === 'keydown' ? 'down KeyC' : type]);
  key('keyup', 'ShiftRight');
  key('keyup', 'ControlLeft');
  stop();
});

it('handles a standalone modifier and re-arms a physically held modifier after intercepted paste', () => {
  const { canvas, sent, key, stop } = pausedCanvas();
  key('keydown', 'AltRight', { altKey: true });
  key('keyup', 'AltRight');
  expect(sent).toEqual(['down AltRight', 'up AltRight']);
  sent.length = 0;
  key('keydown', 'ControlRight', { ctrlKey: true });
  canvas.dispatchEvent(new Event('rdp-release-keys'));
  expect(sent).toEqual([]);
  key('keydown', 'KeyC', { ctrlKey: true });
  expect(sent).toEqual(['down ControlRight', 'down KeyC']);
  key('keyup', 'KeyC', { ctrlKey: true });
  key('keyup', 'ControlRight');
  stop();
});

it('leaves the first wake to clipboard acknowledgement when the paused user presses Ctrl+V', () => {
  vi.stubGlobal('KeyboardEvent', Key);
  const target = Object.assign(new EventTarget(), { tagName: 'CANVAS' });
  const toRemote = vi.fn(async () => {});
  const stopPaste = subscribeClipboardInput(target as unknown as Window, new EventTarget() as Document,
    'bidirectional', () => true, { toRemote });
  const down = vi.fn();
  const up = vi.fn();
  const stopKeys = subscribeCanvasKeyboard(target, null, down, up, null, { canvas: target, isPaused: () => true });
  target.dispatchEvent(new Key('keydown', { code: 'ControlLeft', ctrlKey: true }));
  target.dispatchEvent(new Key('keydown', { code: 'KeyV', ctrlKey: true }));
  target.dispatchEvent(new Key('keyup', { code: 'KeyV', ctrlKey: true }));
  target.dispatchEvent(new Key('keyup', { code: 'ControlLeft' }));
  expect(toRemote).toHaveBeenCalledExactlyOnceWith(true);
  expect(down).not.toHaveBeenCalled();
  expect(up).not.toHaveBeenCalled();
  stopKeys();
  stopPaste();
});

it('leaves the status shortcut to the remote session when auto-hide has no reveal target', () => {
  const target = new EventTarget();
  const down = vi.fn();
  const cleanup = subscribeCanvasKeyboard(target, null, down, vi.fn());
  const chord = new Key('keydown', { code: 'KeyS', ctrlKey: true, altKey: true, shiftKey: true });
  target.dispatchEvent(chord);
  expect(down).toHaveBeenCalledExactlyOnceWith(chord);
  vi.stubGlobal('KeyboardEvent', Key);
  cleanup();
});

it.each(['CANVAS', 'BUTTON'])('reveals status bar by shortcut from %s without forwarding it or changing remote Tab', (tagName) => {
  vi.stubGlobal('KeyboardEvent', Key);
  const target = Object.assign(new EventTarget(), { tagName: 'CANVAS' });
  const down = vi.fn();
  const up = vi.fn();
  const statusBar = Object.assign(new EventTarget(), { focus: vi.fn() });
  const cleanup = subscribeCanvasKeyboard(target, null, down, up, statusBar);
  for (const code of ['ControlLeft', 'AltLeft', 'ShiftLeft']) target.dispatchEvent(new Key('keydown', { code }));
  target.tagName = tagName;
  const shortcut = new Key('keydown', { code: 'KeyS', key: 'S', ctrlKey: true, altKey: true, shiftKey: true });
  target.dispatchEvent(shortcut);
  expect(shortcut.defaultPrevented).toBe(true);
  expect(statusBar.focus).toHaveBeenCalledOnce();
  expect(down).toHaveBeenCalledTimes(3);
  expect(up.mock.calls.map(([event]) => event.code)).toEqual(['ControlLeft', 'AltLeft', 'ShiftLeft']);
  target.dispatchEvent(new Key('keyup', { code: 'KeyS', ctrlKey: true, altKey: true, shiftKey: true }));
  expect(up).toHaveBeenCalledTimes(3);
  target.tagName = 'CANVAS';
  const tab = new Key('keydown', { code: 'Tab', key: 'Tab' });
  target.dispatchEvent(tab);
  expect(down).toHaveBeenLastCalledWith(tab);
  cleanup();
});

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

it.each(['pointerdown', 'pointerover'])('releases remote modifiers on blur and local %s before focus changes', (eventType) => {
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
  target.dispatchEvent(new Event(eventType));
  expect(up.mock.calls.map(([e]) => e.code)).toEqual(['ControlLeft', 'AltLeft']);
  cleanup();
});

it.each(['menu', 'dialog', 'tablist', 'tree'])('isolates portalled/local %s descendants and releases previously forwarded keys', (role) => {
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

it('does not open disconnect for menu-consumed Escape even when its target is BODY', () => {
  const target = Object.assign(new EventTarget(), { tagName: 'BODY' });
  const open = vi.fn();
  const cleanup = subscribeSessionEscape(target, open);
  const escape = new Key('keydown', { key: 'Escape', code: 'Escape' });
  // Radix handles dismissal on document capture, before window listeners.
  escape.preventDefault();
  target.dispatchEvent(escape);
  expect(open).not.toHaveBeenCalled();
  cleanup();
});

it.each([true, false])('unconsumed canvas Escape opens one local prompt, never remote input (session first: %s)', (sessionFirst) => {
  vi.stubGlobal('KeyboardEvent', Key);
  const target = Object.assign(new EventTarget(), { tagName: 'CANVAS' });
  const open = vi.fn();
  const down = vi.fn();
  const up = vi.fn();
  let sessionCleanup = () => {};
  if (sessionFirst) sessionCleanup = subscribeSessionEscape(target, open);
  const canvasCleanup = subscribeCanvasKeyboard(target, null, down, up);
  if (!sessionFirst) sessionCleanup = subscribeSessionEscape(target, open);
  target.dispatchEvent(new Key('keydown', { key: 'Escape', code: 'Escape' }));
  target.dispatchEvent(new Key('keyup', { key: 'Escape', code: 'Escape' }));
  expect(open).toHaveBeenCalledOnce();
  expect(down).not.toHaveBeenCalled();
  expect(up).not.toHaveBeenCalled();
  sessionCleanup();
  canvasCleanup();
  target.dispatchEvent(new Key('keydown', { key: 'Escape', code: 'Escape' }));
  expect(open).toHaveBeenCalledOnce();
});
