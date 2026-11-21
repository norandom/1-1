/**
 * Tells the native layer what the page shows over it (spec
 * rdp-native-renderer, task 5.2): the visible session, where its picture goes
 * in the web view, which part of the remote desktop that is, and the page's
 * background the layer clears to.
 */
import { invoke } from '@tauri-apps/api/core';

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface NativeView {
  /** `null`: no session on screen; the layer shows only the background. */
  session: string | null;
  /** CSS pixels of the web view. */
  dest: Rect;
  /** Remote desktop pixels. */
  source: Rect;
  smooth: boolean;
  background: [number, number, number];
}

type Box = Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>;

/**
 * The visible part of the canvas (clipped to its scroll container) and the
 * matching part of a `surface`-sized remote desktop; `null` when nothing of
 * it is visible.
 */
export function visibleView(
  canvas: Box,
  container: Box,
  surface: { width: number; height: number },
): { dest: Rect; source: Rect } | null {
  const left = Math.max(canvas.left, container.left);
  const top = Math.max(canvas.top, container.top);
  const right = Math.min(canvas.left + canvas.width, container.left + container.width);
  const bottom = Math.min(canvas.top + canvas.height, container.top + container.height);
  if (right <= left || bottom <= top || canvas.width <= 0 || canvas.height <= 0 || surface.width <= 0 || surface.height <= 0) {
    return null;
  }
  const sx = surface.width / canvas.width;
  const sy = surface.height / canvas.height;
  return {
    dest: { x: left, y: top, width: right - left, height: bottom - top },
    source: {
      x: (left - canvas.left) * sx,
      y: (top - canvas.top) * sy,
      width: (right - left) * sx,
      height: (bottom - top) * sy,
    },
  };
}

/** The page background as RGB, resolved by the browser from the theme. */
export function pageBackground(): [number, number, number] {
  try {
    const color = getComputedStyle(document.documentElement).getPropertyValue('--color-surface-0').trim();
    const ctx = document.createElement('canvas').getContext('2d');
    if (!ctx || !color) return [0, 0, 0];
    ctx.fillStyle = color;
    ctx.fillRect(0, 0, 1, 1);
    const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
    return [r, g, b];
  } catch {
    return [0, 0, 0];
  }
}

let lastSent = '';
let lastView: Omit<NativeView, 'background'> | null = null;
let themeWatch: MutationObserver | null = null;

const NO_SESSION: Omit<NativeView, 'background'> = {
  session: null,
  dest: { x: 0, y: 0, width: 0, height: 0 },
  source: { x: 0, y: 0, width: 0, height: 0 },
  smooth: false,
};

/**
 * Sends the view to the backend when it changed; `null` hides the session.
 * The page background goes with every view, so the layer matches the page
 * even with no session open.
 */
export function reportNativeView(view: Omit<NativeView, 'background'> | null): void {
  lastView = view;
  const full: NativeView = { ...(view ?? NO_SESSION), background: pageBackground() };
  const key = JSON.stringify(full);
  if (key === lastSent) return;
  lastSent = key;
  invoke('native_view', { view: full }).catch(() => {});
  // A theme change repaints the page background: follow it.
  if (!themeWatch && typeof MutationObserver !== 'undefined') {
    themeWatch = new MutationObserver(() => reportNativeView(lastView));
    themeWatch.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'data-theme'] });
  }
}

/** Test hook: forget what was sent. */
export function resetNativeViewForTests(): void {
  lastSent = '';
  lastView = null;
  themeWatch?.disconnect();
  themeWatch = null;
}
