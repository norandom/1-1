import { expect, it, vi } from 'vitest';
import { subscribeCanvasWheel } from './canvas-wheel';

it('registers a non-passive wheel listener, cancels before forwarding once, and cleans up across reattachment', () => {
  const canvas = new EventTarget();
  const add = vi.spyOn(canvas, 'addEventListener');
  const remove = vi.spyOn(canvas, 'removeEventListener');
  const forward = vi.fn((event: WheelEvent) => expect(event.defaultPrevented).toBe(true));
  const cleanup = subscribeCanvasWheel(canvas, forward);
  expect(add).toHaveBeenCalledExactlyOnceWith('wheel', expect.any(Function), { passive: false });

  const wheel = new Event('wheel', { cancelable: true });
  expect(canvas.dispatchEvent(wheel)).toBe(false);
  expect(forward).toHaveBeenCalledExactlyOnceWith(wheel);

  cleanup();
  expect(remove).toHaveBeenCalledExactlyOnceWith('wheel', add.mock.calls[0][1]);
  const afterCleanup = new Event('wheel', { cancelable: true });
  expect(canvas.dispatchEvent(afterCleanup)).toBe(true);
  expect(forward).toHaveBeenCalledTimes(1);

  const nextForward = vi.fn();
  const nextCleanup = subscribeCanvasWheel(canvas, nextForward);
  const nextWheel = new Event('wheel', { cancelable: true });
  expect(canvas.dispatchEvent(nextWheel)).toBe(false);
  expect(nextForward).toHaveBeenCalledExactlyOnceWith(nextWheel);
  expect(forward).toHaveBeenCalledTimes(1);
  nextCleanup();
});
