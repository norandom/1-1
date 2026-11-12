/**
 * Trailing-edge debounce for remote resolution changes, extracted from the
 * session view's resize handler: every call supersedes the pending request
 * and (re)arms the timer; only the latest size is emitted, once, after the
 * debounce window with no further calls.
 */
export const RESIZE_DEBOUNCE_MS = 200;

export interface ResizeScheduler {
  schedule(width: number, height: number): void;
  cancel(): void;
}

export function createTrailingResizeScheduler(
  emit: (width: number, height: number) => void,
  delayMs: number = RESIZE_DEBOUNCE_MS
): ResizeScheduler {
  let timer: ReturnType<typeof setTimeout> | null = null;

  return {
    schedule(width: number, height: number) {
      if (timer !== null) {
        clearTimeout(timer);
      }
      timer = setTimeout(() => {
        timer = null;
        emit(width, height);
      }, delayMs);
    },
    cancel() {
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
    },
  };
}
