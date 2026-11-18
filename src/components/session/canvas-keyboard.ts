import { isDisplayModeShortcut, releaseForwardedKeys } from '../../lib/input-mapper';
import { matchKeyboardShortcut } from '../../lib/shortcuts';

const modifiers = [
  ['ctrlKey', 'ControlLeft', 'ControlRight'], ['shiftKey', 'ShiftLeft', 'ShiftRight'],
  ['altKey', 'AltLeft', 'AltRight'], ['metaKey', 'MetaLeft', 'MetaRight'],
] as const;
const modifierCodes = new Set<string>(modifiers.flatMap(([, left, right]) => [left, right]));

export function subscribeSessionEscape(target: EventTarget, openDisconnect: () => void): () => void {
  const handleKey = (event: Event) => {
    const e = event as KeyboardEvent;
    if (e.key !== 'Escape' || isLocalInputTarget(e.target)) return;
    if (e.defaultPrevented) return;
    e.preventDefault();
    openDisconnect();
  };
  target.addEventListener('keydown', handleKey);
  return () => target.removeEventListener('keydown', handleKey);
}

/** Local UI includes portalled Radix menus and dialogs, not only form inputs. */
export function isLocalInputTarget(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null;
  return !!element && (['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON', 'A', 'SUMMARY'].includes(element.tagName)
    || !!element.closest?.('[data-local-input], [role="menu"], [role="dialog"], [role="tablist"], [role="tree"], [contenteditable="true"]'));
}

/** Own the remote key lifecycle and the local display-control escape. */
export function subscribeCanvasKeyboard(
  target: EventTarget,
  select: (EventTarget & { focus(): void }) | null,
  onKeyDown: (event: KeyboardEvent) => void,
  onKeyUp: (event: KeyboardEvent) => void,
  paused?: { isPaused: () => boolean; canvas: EventTarget | null },
): () => void {
  const forwardedKeys = new Set<string>();
  const deferred = new Map<string, KeyboardEvent>();
  const physicalModifiers = new Set<string>();
  let reconcileModifiers = false;
  const releaseKeys = () => {
    deferred.clear();
    reconcileModifiers = true;
    releaseForwardedKeys(forwardedKeys, (code) => onKeyUp(new KeyboardEvent('keyup', { code })));
  };
  const forward = (e: KeyboardEvent) => {
    forwardedKeys.add(e.code);
    onKeyDown(e);
  };
  const prepareRemoteInput = (e: KeyboardEvent | MouseEvent, currentCode?: string) => {
    for (const key of deferred.values()) forward(key);
    deferred.clear();
    if (!reconcileModifiers) return;
    // Local menus and clipboard paste can release remote keys while the physical
    // modifier remains held. Restore its original side on the next remote input.
    for (const [flag, left, right] of modifiers) {
      if (!e[flag] || currentCode === left || currentCode === right) continue;
      const sides = [left, right].filter((code) => physicalModifiers.has(code));
      for (const code of sides.length ? sides : [left]) {
        if (!forwardedKeys.has(code)) forward(new KeyboardEvent('keydown', { code }));
      }
    }
    reconcileModifiers = false;
  };
  const handleDown = (event: Event) => {
    const e = event as KeyboardEvent;
    if (modifierCodes.has(e.code)) physicalModifiers.add(e.code);
    if (isLocalInputTarget(e.target)) { releaseKeys(); return; }
    // Escape belongs to the session prompt or the currently open local menu.
    // Do not consume it here: the session listener must work in either order.
    if (e.key === 'Escape' || e.defaultPrevented || matchKeyboardShortcut(e)) {
      releaseKeys();
      return;
    }
    if (isDisplayModeShortcut(e)) {
      e.preventDefault();
      releaseKeys();
      select?.focus();
      return;
    }
    e.preventDefault();
    if (modifierCodes.has(e.code) && paused?.isPaused()) {
      if (!e.repeat && !forwardedKeys.has(e.code)) deferred.set(e.code, e);
      return;
    }
    prepareRemoteInput(e, e.code);
    forward(e);
  };

  const handleUp = (event: Event) => {
    const e = event as KeyboardEvent;
    physicalModifiers.delete(e.code);
    if (isLocalInputTarget(e.target)) { releaseKeys(); return; }
    // A modifier tap is a deliberate remote input. A consumed local chord or
    // paste has already discarded its deferred prefix and owns its keyups.
    if (deferred.has(e.code)) prepareRemoteInput(e, e.code);
    if (!forwardedKeys.delete(e.code)) return;
    e.preventDefault();
    onKeyUp(e);
  };
  const pointer = (e: Event) => prepareRemoteInput(e as MouseEvent);
  const blur = () => { physicalModifiers.clear(); releaseKeys(); };

  target.addEventListener('keydown', handleDown);
  target.addEventListener('keyup', handleUp);
  select?.addEventListener('focus', releaseKeys);
  const enterLocal = (event: Event) => { if (isLocalInputTarget(event.target)) releaseKeys(); };
  target.addEventListener('pointerdown', enterLocal, true);
  target.addEventListener('pointerover', enterLocal, true);
  target.addEventListener('focusin', enterLocal, true);
  target.addEventListener('blur', blur);
  target.addEventListener('rdp-release-keys', releaseKeys);
  paused?.canvas?.addEventListener('mousedown', pointer, true);
  paused?.canvas?.addEventListener('wheel', pointer, true);

  return () => {
    target.removeEventListener('keydown', handleDown);
    target.removeEventListener('keyup', handleUp);
    select?.removeEventListener('focus', releaseKeys);
    target.removeEventListener('pointerdown', enterLocal, true);
    target.removeEventListener('pointerover', enterLocal, true);
    target.removeEventListener('focusin', enterLocal, true);
    target.removeEventListener('blur', blur);
    target.removeEventListener('rdp-release-keys', releaseKeys);
    paused?.canvas?.removeEventListener('mousedown', pointer, true);
    paused?.canvas?.removeEventListener('wheel', pointer, true);
    releaseKeys();
  };
}
