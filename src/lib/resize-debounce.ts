/**
 * Trailing-edge debounce for remote resolution changes, extracted from the
 * session view's resize handler: every call supersedes the pending request
 * and (re)arms the timer; only the latest size is emitted, once, after the
 * debounce window with no further calls.
 *
 * Every remote resize makes the server repaint the whole desktop. A bar or
 * panel that slides in and out again (the status bar reveal, task 11.0g)
 * must not cost that: the window is 1 s (it was 200 ms until 21 November
 * 2026), and a request for the size last sent cancels the pending one
 * instead of sending it again.
 */
export const RESIZE_DEBOUNCE_MS = 1000;

export interface ResizeScheduler {
  schedule(width: number, height: number): void;
  cancel(): void;
}

export function createTrailingResizeScheduler(
  emit: (width: number, height: number) => void,
  delayMs: number = RESIZE_DEBOUNCE_MS,
  enabled: () => boolean = () => true,
  initialSize?: { width: number; height: number },
): ResizeScheduler {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let sent: { width: number; height: number } | null = null;

  const scheduler: ResizeScheduler = {
    schedule(width: number, height: number) {
      if (timer !== null) {
        clearTimeout(timer);
      }
      timer = null;
      if (!enabled()) return;
      // Back to the size the remote already has: nothing to send.
      if (sent && sent.width === width && sent.height === height) return;
      timer = setTimeout(() => {
        timer = null;
        if (enabled()) {
          sent = { width, height };
          emit(width, height);
        }
      }, delayMs);
    },
    cancel() {
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
    },
  };
  // Installation owns the initial request: a child ResizeObserver may already
  // have reported and deduplicated this size before the parent's effect ran.
  if (initialSize) scheduler.schedule(initialSize.width, initialSize.height);
  return scheduler;
}
