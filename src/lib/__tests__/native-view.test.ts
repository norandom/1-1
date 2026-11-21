import { afterEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn(() => Promise.resolve());
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...(args as [])) }));

import { reportNativeView, resetNativeViewForTests, visibleView } from '../native-view';

const box = (left: number, top: number, width: number, height: number) => ({ left, top, width, height });

describe('native view geometry', () => {
  it('fit mode: the whole picture where the canvas is', () => {
    const view = visibleView(box(300, 100, 960, 540), box(260, 80, 1040, 700), { width: 1920, height: 1080 });
    expect(view).toEqual({
      dest: { x: 300, y: 100, width: 960, height: 540 },
      source: { x: 0, y: 0, width: 1920, height: 1080 },
    });
  });

  it('native mode, scrolled: only the visible part, in remote pixels', () => {
    // A 1920x1080 desktop at 1:1 CSS pixels in an 800x600 container scrolled by (400, 300).
    const view = visibleView(box(-400, -250, 1920, 1080), box(0, 50, 800, 600), { width: 1920, height: 1080 });
    expect(view).toEqual({
      dest: { x: 0, y: 50, width: 800, height: 600 },
      source: { x: 400, y: 300, width: 800, height: 600 },
    });
  });

  it('device pixel ratio: CSS size half the remote size', () => {
    const view = visibleView(box(0, 0, 960, 540), box(0, 0, 960, 540), { width: 1920, height: 1080 });
    expect(view?.source).toEqual({ x: 0, y: 0, width: 1920, height: 1080 });
  });

  it('nothing visible, or no picture yet', () => {
    expect(visibleView(box(1000, 0, 100, 100), box(0, 0, 800, 600), { width: 100, height: 100 })).toBeNull();
    expect(visibleView(box(0, 0, 100, 100), box(0, 0, 800, 600), { width: 0, height: 0 })).toBeNull();
  });
});

describe('reporting the view', () => {
  afterEach(() => {
    resetNativeViewForTests();
    invoke.mockClear();
  });

  const view = {
    session: 's1',
    dest: { x: 0, y: 0, width: 10, height: 10 },
    source: { x: 0, y: 0, width: 10, height: 10 },
    smooth: false,
  };

  it('sends a view once, again only after a change, and null to hide', () => {
    reportNativeView(view);
    reportNativeView({ ...view });
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenLastCalledWith('native_view', { view: expect.objectContaining({ session: 's1', smooth: false }) });
    reportNativeView({ ...view, smooth: true });
    expect(invoke).toHaveBeenCalledTimes(2);
    reportNativeView(null);
    expect(invoke).toHaveBeenLastCalledWith('native_view', {
      view: expect.objectContaining({ session: null, background: expect.any(Array) }),
    });
  });
});
