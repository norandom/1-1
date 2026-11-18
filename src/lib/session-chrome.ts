/**
 * Shared visibility of the session overlays: the top toolbar and, for hosts
 * that choose it, the bottom control bar. Both appear when the pointer nears
 * the top edge and hide together after a pause, unless one is hovered,
 * focused or has an open menu.
 */

/** Pointer distance from the top of the window that reveals the overlays. */
export const CHROME_REVEAL_EDGE = 60;
/** Overlays hide this long after the last reveal without a hold. */
export const CHROME_IDLE_MS = 3000;
/** Overlays hide this long after the last hold ends. */
export const CHROME_LEAVE_MS = 1500;

/** Reasons the overlays must stay up. Each is set and cleared idempotently. */
export type ChromeHold = 'toolbar-hover' | 'bar-hover' | 'bar-focus' | 'bar-menu';

export interface ChromeTimers {
  set(callback: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

const browserTimers: ChromeTimers = {
  set: (callback, ms) => setTimeout(callback, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export function isChromeRevealPoint(clientY: number): boolean {
  return clientY < CHROME_REVEAL_EDGE;
}

export function createChromeVisibility(publish: (visible: boolean) => void, timers: ChromeTimers = browserTimers) {
  const holds = new Set<ChromeHold>();
  let visible = false;
  let timer: unknown;
  const apply = (next: boolean) => {
    if (visible === next) return;
    visible = next;
    publish(next);
  };
  const cancel = () => {
    if (timer !== undefined) timers.clear(timer);
    timer = undefined;
  };
  const hideAfter = (ms: number) => {
    cancel();
    timer = timers.set(() => {
      timer = undefined;
      if (holds.size === 0) apply(false);
    }, ms);
  };
  return {
    /** The pointer reached the reveal edge. */
    reveal() {
      apply(true);
      if (holds.size === 0) hideAfter(CHROME_IDLE_MS);
    },
    hold(reason: ChromeHold) {
      holds.add(reason);
      cancel();
      apply(true);
    },
    release(reason: ChromeHold) {
      if (!holds.delete(reason)) return;
      if (holds.size === 0 && visible) hideAfter(CHROME_LEAVE_MS);
    },
    /** Leaving the session view: hide at once and forget every hold. */
    reset() {
      holds.clear();
      cancel();
      apply(false);
    },
  };
}
