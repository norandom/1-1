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

  it('debounces at the pinned 1 s window', () => {
    const emit = vi.fn();
    const scheduler = createTrailingResizeScheduler(emit);

    scheduler.schedule(640, 480);
    expect(emit).not.toHaveBeenCalled();

    // Literals pin the absolute window; referencing the constant would
    // let a timing regression pass unnoticed.
    vi.advanceTimersByTime(999);
    expect(emit).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(emit).toHaveBeenCalledTimes(1);
    expect(emit).toHaveBeenCalledWith(640, 480);
    expect(RESIZE_DEBOUNCE_MS).toBe(1000);
  });

  it('coalesces bursts: only the latest size is emitted, once', () => {
    const emit = vi.fn();
    const scheduler = createTrailingResizeScheduler(emit);

    scheduler.schedule(800, 600);
    vi.advanceTimersByTime(600);
    scheduler.schedule(1024, 768);
    vi.advanceTimersByTime(600);
    scheduler.schedule(1280, 720);

    // Each call resets the window; 1 s after the LAST call…
    vi.advanceTimersByTime(1000);
    expect(emit).toHaveBeenCalledTimes(1);
    expect(emit).toHaveBeenCalledWith(1280, 720);
  });

  it('re-arms after emitting: a later resize debounces again', () => {
    const emit = vi.fn();
    const scheduler = createTrailingResizeScheduler(emit);

    scheduler.schedule(640, 480);
    vi.advanceTimersByTime(1000);
    scheduler.schedule(800, 600);
    vi.advanceTimersByTime(1000);

    expect(emit).toHaveBeenCalledTimes(2);
    expect(emit).toHaveBeenNthCalledWith(1, 640, 480);
    expect(emit).toHaveBeenNthCalledWith(2, 800, 600);
  });

  it('cancel() suppresses a pending emission', () => {
    const emit = vi.fn();
    const scheduler = createTrailingResizeScheduler(emit);

    scheduler.schedule(640, 480);
    scheduler.cancel();
    vi.advanceTimersByTime(2000);

    expect(emit).not.toHaveBeenCalled();
  });

  it('a panel sliding in and out again costs no remote resize', () => {
    const emit = vi.fn();
    const scheduler = createTrailingResizeScheduler(emit);
    scheduler.schedule(1600, 900);
    vi.advanceTimersByTime(1000);
    expect(emit).toHaveBeenCalledExactlyOnceWith(1600, 900);

    // The status bar slides in (the area shrinks) and out again in time.
    scheduler.schedule(1600, 872);
    vi.advanceTimersByTime(700);
    scheduler.schedule(1600, 900);
    vi.advanceTimersByTime(5000);
    expect(emit).toHaveBeenCalledTimes(1);

    // A bar that stays shown does resize, once, after the quiet second.
    scheduler.schedule(1600, 872);
    vi.advanceTimersByTime(1000);
    expect(emit).toHaveBeenLastCalledWith(1600, 872);
    expect(emit).toHaveBeenCalledTimes(2);
  });

  it('seeds a newly installed scheduler without relying on another observer notification', () => {
    const emit = vi.fn();
    let dynamic = true;
    const first = createTrailingResizeScheduler(emit, 200, () => dynamic, { width: 1200, height: 800 });
    // Fixed mode removes the old scheduler, including its seed request.
    dynamic = false;
    first.cancel();
    vi.advanceTimersByTime(200);
    expect(emit).not.toHaveBeenCalled();
    dynamic = true;
    const resumed = createTrailingResizeScheduler(emit, 200, () => dynamic, { width: 1600, height: 900 });
    // The child observer already reported this same size before installation.
    // It need not report again for fixed -> dynamic to take effect.
    vi.advanceTimersByTime(200);
    expect(emit).toHaveBeenCalledExactlyOnceWith(1600, 900);
    resumed.cancel();
  });
});

it('does not send a queued resize after switching to fixed, and resumes with a fresh measured size', () => {
  vi.useFakeTimers();
  let dynamic = true;
  const emit = vi.fn();
  const scheduler = createTrailingResizeScheduler(emit, 200, () => dynamic);
  scheduler.schedule(800, 600);
  dynamic = false;
  vi.advanceTimersByTime(200);
  scheduler.schedule(900, 700);
  vi.advanceTimersByTime(200);
  expect(emit).not.toHaveBeenCalled();
  dynamic = true;
  scheduler.schedule(1200, 800);
  vi.advanceTimersByTime(200);
  expect(emit).toHaveBeenCalledExactlyOnceWith(1200, 800);
  scheduler.cancel();
  vi.useRealTimers();
});
