import { isDisplayModeShortcut, releaseForwardedKeys } from '../../lib/input-mapper';

/** Local UI includes portalled Radix menus and dialogs, not only form inputs. */
export function isLocalInputTarget(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null;
  return !!element && (['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON', 'A', 'SUMMARY'].includes(element.tagName)
    || !!element.closest?.('[data-local-input], [role="menu"], [role="dialog"], [role="tablist"], [contenteditable="true"]'));
}

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
    if (isLocalInputTarget(e.target)) { releaseKeys(); return; }
    if (isDisplayModeShortcut(e)) {
      e.preventDefault();
      releaseKeys();
      select?.focus();
      return;
    }
    e.preventDefault();
    forwardedKeys.add(e.code);
    onKeyDown(e);
  };

  const handleUp = (event: Event) => {
    const e = event as KeyboardEvent;
    if (isLocalInputTarget(e.target)) { releaseKeys(); return; }
    if (isDisplayModeShortcut(e)) return;
    e.preventDefault();
    forwardedKeys.delete(e.code);
    onKeyUp(e);
  };

  target.addEventListener('keydown', handleDown);
  target.addEventListener('keyup', handleUp);
  select?.addEventListener('focus', releaseKeys);
  const enterLocal = (event: Event) => { if (isLocalInputTarget(event.target)) releaseKeys(); };
  target.addEventListener('pointerdown', enterLocal, true);
  target.addEventListener('focusin', enterLocal, true);
  target.addEventListener('blur', releaseKeys);

  return () => {
    target.removeEventListener('keydown', handleDown);
    target.removeEventListener('keyup', handleUp);
    select?.removeEventListener('focus', releaseKeys);
    target.removeEventListener('pointerdown', enterLocal, true);
    target.removeEventListener('focusin', enterLocal, true);
    target.removeEventListener('blur', releaseKeys);
    releaseKeys();
  };
}
