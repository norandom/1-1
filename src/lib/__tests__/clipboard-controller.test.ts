import { expect, it, vi } from 'vitest';
import { createClipboardController } from '../clipboard-controller';

it('rejects oversized screenshot payloads before publishing them', async () => {
  const state = vi.fn(); const publish = vi.fn(async () => {});
  const controller = createClipboardController({ direction: 'bidirectional', read: async () => ({ png: 'A'.repeat(24 * 1024 * 1024) }), write: async () => {}, publish, request: async () => {}, paste: async () => {}, state });
  await controller.toRemote();
  expect(state).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'error' }));
  expect(publish).not.toHaveBeenCalled();
});

it('keeps repeated text and screenshot transfers distinct and respects image direction policy', async () => {
  const image = { png: 'iVBORw0KGgo=' };
  const values = ['before', image, 'after'];
  const publish = vi.fn(async () => {}); const write = vi.fn(async () => {});
  const controller = createClipboardController({ direction: 'bidirectional', read: async () => values.shift()!, write, publish, request: async () => {}, paste: async () => {}, state: vi.fn() });
  for (let i = 0; i < 3; i++) await controller.toRemote();
  expect(publish.mock.calls).toEqual([['before'], [image], ['after']]);
  await controller.receive(image); await controller.receive('text again');
  expect(write.mock.calls).toEqual([[image], ['text again']]);
  const disabled = createClipboardController({ direction: 'off', read: async () => image, write, publish, request: async () => {}, paste: async () => {}, state: vi.fn() });
  await disabled.toRemote(); await disabled.receive(image);
  expect(publish).toHaveBeenCalledTimes(3); expect(write).toHaveBeenCalledTimes(2);
});
import { subscribeClipboardInput } from '../clipboard-input';
import { subscribeCanvasKeyboard } from '../../components/session/canvas-keyboard';

it('waits for remote acknowledgement before paste and discards stale local reads', async () => {
  let accept!: () => void;
  const publish = vi.fn(() => new Promise<void>((resolve) => { accept = resolve; }));
  const paste = vi.fn(async () => {});
  const controller = createClipboardController({ direction: 'bidirectional', read: async () => 'fresh', write: async () => {}, publish, request: async () => {}, paste, state: vi.fn() });
  const work = controller.toRemote(true);
  await vi.waitFor(() => expect(publish).toHaveBeenCalledWith('fresh'));
  expect(paste).not.toHaveBeenCalled();
  accept(); await work;
  expect(paste).toHaveBeenCalledOnce();
  let read!: (text: string) => void;
  const old = createClipboardController({ direction: 'bidirectional', read: () => new Promise((resolve) => { read = resolve; }), write: async () => {}, publish, request: async () => {}, paste, state: vi.fn() });
  const late = old.toRemote(); old.dispose(); read('private'); await late;
  expect(publish).toHaveBeenCalledTimes(1);
});

it('enforces direction and surfaces clipboard denial without forwarding paste', async () => {
  const paste = vi.fn(async () => {}); const state = vi.fn(); const publish = vi.fn(async () => {});
  const controller = createClipboardController({ direction: 'localToRemote', read: async () => { throw new Error('Denied'); }, write: async () => {}, publish, request: async () => {}, paste, state });
  await controller.toRemote(true);
  expect(paste).not.toHaveBeenCalled();
  expect(state).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'error' }));
  await controller.fromRemote();
  expect(state).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'error' }));
});

it('times out a local read and never publishes its late result', async () => {
  vi.useFakeTimers();
  let resolve!: (text: string) => void;
  const publish = vi.fn(async () => {}); const state = vi.fn();
  const controller = createClipboardController({ direction: 'bidirectional', read: () => new Promise((done) => { resolve = done; }), write: async () => {}, publish, request: async () => {}, paste: async () => {}, state });
  const work = controller.toRemote(true);
  await vi.advanceTimersByTimeAsync(10000); await work;
  expect(state).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'error' }));
  resolve('late'); await Promise.resolve();
  expect(publish).not.toHaveBeenCalled(); controller.dispose(); vi.useRealTimers();
});

it('requires local application of an explicit remote transfer and reports missing delivery', async () => {
  vi.useFakeTimers(); const state = vi.fn(); const write = vi.fn(async () => {});
  const controller = createClipboardController({ direction: 'bidirectional', read: async () => '', write, publish: async () => {}, request: async () => {}, paste: async () => {}, state });
  const success = controller.fromRemote(); await controller.receive('first'); await success;
  expect(write).toHaveBeenCalledExactlyOnceWith('first');
  expect(state).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'copied' }));
  const missing = controller.fromRemote(); await vi.advanceTimersByTimeAsync(10000); await missing;
  expect(state).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'error' }));
  controller.dispose(); vi.useRealTimers();
});

it('serializes remote writes, discards queued stale data and fences disposed owners', async () => {
  let complete!: () => void; const state = vi.fn();
  const write = vi.fn(() => new Promise<void>((resolve) => { complete = resolve; }));
  const controller = createClipboardController({ direction: 'remoteToLocal', read: async () => '', write, publish: async () => {}, request: async () => {}, paste: async () => {}, state });
  const first = controller.receive('old'); await vi.waitFor(() => expect(write).toHaveBeenCalledOnce());
  const second = controller.receive('queued'); controller.dispose(); complete(); await Promise.all([first, second]);
  expect(write).toHaveBeenCalledTimes(1); expect(state).not.toHaveBeenCalled();
  const denied = createClipboardController({ direction: 'localToRemote', read: async () => '', write, publish: async () => {}, request: async () => {}, paste: async () => {}, state });
  await denied.receive('not allowed'); expect(write).toHaveBeenCalledTimes(1);
});

it('does not replace clipboard while local UI is focused unless resync was explicitly requested', async () => {
  const write = vi.fn(async () => {});
  const controller = createClipboardController({ direction: 'bidirectional', read: async () => '', write, publish: async () => {}, request: async () => {}, paste: async () => {}, state: vi.fn(), allowIncoming: () => false });
  await controller.receive('background'); expect(write).not.toHaveBeenCalled();
  const requested = controller.fromRemote(); await controller.receive('requested'); await requested;
  expect(write).toHaveBeenCalledExactlyOnceWith('requested');
});

it('a replaced controller cannot publish content or status into the new session owner', async () => {
  let read!: (text: string) => void; const publish = vi.fn(async () => {});
  let status = '';
  const old = createClipboardController({ direction: 'bidirectional', read: () => new Promise((resolve) => { read = resolve; }), write: async () => {}, publish, request: async () => {}, paste: async () => {}, state: (value) => { status = value.message; } });
  const pending = old.toRemote(); old.dispose();
  const current = createClipboardController({ direction: 'off', read: async () => '', write: async () => {}, publish, request: async () => {}, paste: async () => {}, state: (value) => { status = value.message; } });
  await current.toRemote(); const newStatus = status;
  read('old session'); await pending;
  expect(publish).not.toHaveBeenCalled(); expect(status).toBe(newStatus);
});

class Key extends Event {
  code = 'KeyV'; key = 'v'; ctrlKey = true; metaKey = false; shiftKey = false; altKey = false; repeat = false;
  constructor(type: string, values: Record<string, unknown> = {}) { super(type, { cancelable: true }); Object.assign(this, values); }
}
it('captures remote paste, releases held modifiers, suppresses keyup and leaves local fields alone', async () => {
  vi.stubGlobal('KeyboardEvent', Key);
  const target = Object.assign(new EventTarget(), { tagName: 'CANVAS' });
  const documentTarget = new EventTarget(); const paste = vi.fn(async () => {}); const down = vi.fn(); const up = vi.fn();
  const stopClipboard = subscribeClipboardInput(target as unknown as Window, documentTarget as Document, 'bidirectional', () => true, { toRemote: paste });
  const stopKeys = subscribeCanvasKeyboard(target, null, down, up);
  target.dispatchEvent(new Key('keydown', { code: 'ControlLeft', key: 'Control' }));
  const event = new Key('keydown'); target.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(true); expect(paste).toHaveBeenCalledExactlyOnceWith(true);
  expect(up.mock.calls.map(([key]) => key.code)).toEqual(['ControlLeft']);
  const nativePaste = Object.assign(new Event('paste', { cancelable: true }), {});
  Object.defineProperty(nativePaste, 'target', { value: target });
  documentTarget.dispatchEvent(nativePaste); expect(paste).toHaveBeenCalledTimes(1);
  target.dispatchEvent(new Key('keyup')); expect(up).toHaveBeenCalledTimes(1);
  target.dispatchEvent(new Key('keydown', { code: 'KeyC', key: 'c' }));
  expect(down.mock.calls.slice(-2).map(([key]) => key.code)).toEqual(['ControlLeft', 'KeyC']);
  target.tagName = 'INPUT'; target.dispatchEvent(new Key('keydown')); expect(paste).toHaveBeenCalledTimes(1);
  stopClipboard(); stopKeys(); vi.unstubAllGlobals();
});
