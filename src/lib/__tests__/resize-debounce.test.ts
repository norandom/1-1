import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  RESIZE_DEBOUNCE_MS,
  createTrailingResizeScheduler,
} from '../resize-debounce';

describe('createTrailingResizeScheduler (Req 3.3)', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('debounces at the pinned 200 ms window', () => {
    const emit = vi.fn();
    const scheduler = createTrailingResizeScheduler(emit);

    scheduler.schedule(640, 480);
    expect(emit).not.toHaveBeenCalled();

    // Literals pin the absolute window; referencing the constant would
    // let a timing regression pass unnoticed.
    vi.advanceTimersByTime(199);
    expect(emit).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(emit).toHaveBeenCalledTimes(1);
    expect(emit).toHaveBeenCalledWith(640, 480);
    expect(RESIZE_DEBOUNCE_MS).toBe(200);
  });

  it('coalesces bursts: only the latest size is emitted, once', () => {
    const emit = vi.fn();
    const scheduler = createTrailingResizeScheduler(emit);

    scheduler.schedule(800, 600);
    vi.advanceTimersByTime(120);
    scheduler.schedule(1024, 768);
    vi.advanceTimersByTime(120);
    scheduler.schedule(1280, 720);

    // Each call resets the window; 200 ms after the LAST call…
    vi.advanceTimersByTime(200);
    expect(emit).toHaveBeenCalledTimes(1);
    expect(emit).toHaveBeenCalledWith(1280, 720);
  });

  it('re-arms after emitting: a later resize debounces again', () => {
    const emit = vi.fn();
    const scheduler = createTrailingResizeScheduler(emit);

    scheduler.schedule(640, 480);
    vi.advanceTimersByTime(200);
    scheduler.schedule(800, 600);
    vi.advanceTimersByTime(200);

    expect(emit).toHaveBeenCalledTimes(2);
    expect(emit).toHaveBeenNthCalledWith(1, 640, 480);
    expect(emit).toHaveBeenNthCalledWith(2, 800, 600);
  });

  it('cancel() suppresses a pending emission', () => {
    const emit = vi.fn();
    const scheduler = createTrailingResizeScheduler(emit);

    scheduler.schedule(640, 480);
    scheduler.cancel();
    vi.advanceTimersByTime(400);

    expect(emit).not.toHaveBeenCalled();
  });
});
