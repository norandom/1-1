import { isDisplayModeShortcut, releaseForwardedKeys } from '../../lib/input-mapper';

/** Own the remote key lifecycle and the local display-control escape. */
export function subscribeCanvasKeyboard(
  target: EventTarget,
  select: (EventTarget & { focus(): void }) | null,
  onKeyDown: (event: KeyboardEvent) => void,
  onKeyUp: (event: KeyboardEvent) => void,
): () => void {
  const forwardedKeys = new Set<string>();
  const releaseKeys = () => releaseForwardedKeys(forwardedKeys, (code) =>
    onKeyUp(new KeyboardEvent('keyup', { code })));
  const handleDown = (event: Event) => {
    const e = event as KeyboardEvent;
    if (isDisplayModeShortcut(e)) {
      e.preventDefault();
      releaseKeys();
      select?.focus();
      return;
    }
    // Only capture when the session view is active (not typing in a dialog/input)
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
    e.preventDefault();
    forwardedKeys.add(e.code);
    onKeyDown(e);
  };

  const handleUp = (event: Event) => {
    const e = event as KeyboardEvent;
    const tag = (e.target as HTMLElement)?.tagName;
    if (isDisplayModeShortcut(e)) return;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
    e.preventDefault();
    forwardedKeys.delete(e.code);
    onKeyUp(e);
  };

  target.addEventListener('keydown', handleDown);
  target.addEventListener('keyup', handleUp);
  select?.addEventListener('focus', releaseKeys);

  return () => {
    target.removeEventListener('keydown', handleDown);
    target.removeEventListener('keyup', handleUp);
    select?.removeEventListener('focus', releaseKeys);
    releaseKeys();
  };
}
