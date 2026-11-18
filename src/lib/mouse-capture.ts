/** A local position in client coordinates, as reported by a mouse event. */
export interface ClientPoint {
  clientX: number;
  clientY: number;
}

/** A remote button press that still needs its release. */
export interface PendingRelease extends ClientPoint {
  button: number;
}

/**
 * Tracks the mouse buttons the remote desktop has seen pressed. A drag can end
 * where the canvas never receives the release (focus loss, a lost capture), so
 * the remote would keep the button held and a dragged window would follow the
 * pointer. Drain the remaining presses to send their releases explicitly.
 */
export function createHeldButtons() {
  const held = new Set<number>();
  let last: ClientPoint = { clientX: 0, clientY: 0 };
  const at = (point: ClientPoint) => {
    last = { clientX: point.clientX, clientY: point.clientY };
  };
  return {
    move: at,
    press(button: number, point: ClientPoint) {
      held.add(button);
      at(point);
    },
    release(button: number, point: ClientPoint) {
      held.delete(button);
      at(point);
    },
    get pressed() {
      return held.size > 0;
    },
    /** Releases for every button still held, at the last known position. */
    drain(): PendingRelease[] {
      const releases = [...held].map((button) => ({ button, ...last }));
      held.clear();
      return releases;
    },
  };
}
