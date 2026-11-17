/** Canvas wheels belong to the remote desktop; scrollbars pan the viewport. */
export function subscribeCanvasWheel(canvas: EventTarget, forward: (event: WheelEvent) => void): () => void {
  const listener = (event: Event) => {
    event.preventDefault();
    forward(event as WheelEvent);
  };
  canvas.addEventListener('wheel', listener, { passive: false });
  return () => canvas.removeEventListener('wheel', listener);
}
