import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

import { useToastStore } from '../toastStore';

describe('toastStore', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    useToastStore.setState({ toasts: [] });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('adds toasts with the 3s default duration, newest first', () => {
    const { addToast } = useToastStore.getState();

    addToast({ message: 'first', type: 'info' });
    addToast({ message: 'second', type: 'success' });

    const toasts = useToastStore.getState().toasts;
    expect(toasts.map((t) => t.message)).toEqual(['second', 'first']);
    expect(toasts.every((t) => t.duration === 3000)).toBe(true);
    expect(toasts[0].id).not.toBe(toasts[1].id);
  });

  it('auto-dismisses a default toast after exactly 3000 ms', () => {
    useToastStore.getState().addToast({ message: 'hello', type: 'info' });

    vi.advanceTimersByTime(2999);
    expect(useToastStore.getState().toasts).toHaveLength(1);

    vi.advanceTimersByTime(1);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it('honors a custom duration', () => {
    useToastStore.getState().addToast({ message: 'custom', type: 'warning', duration: 250 });

    vi.advanceTimersByTime(249);
    expect(useToastStore.getState().toasts).toHaveLength(1);

    vi.advanceTimersByTime(1);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it('keeps duration-0 toasts on screen (sticky)', () => {
    useToastStore.getState().addToast({ message: 'sticky', type: 'error', duration: 0 });

    vi.advanceTimersByTime(60_000);
    expect(useToastStore.getState().toasts).toHaveLength(1);
  });

  it('removes only the targeted toast', () => {
    const { addToast, removeToast } = useToastStore.getState();
    addToast({ message: 'a', type: 'info' });
    addToast({ message: 'b', type: 'info' });
    const firstId = useToastStore.getState().toasts[1].id;

    removeToast(firstId);

    expect(useToastStore.getState().toasts.map((t) => t.message)).toEqual(['b']);
  });
});
