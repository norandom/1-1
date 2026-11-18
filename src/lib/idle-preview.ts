import type { SessionInfo } from '../types';

export const IDLE_PREVIEW_LABEL = 'Idle preview after 5 minutes';
export const IDLE_PREVIEW_HELP = 'On supported hosts, a busy desktop can refresh about once a minute while you are away. Input resumes live updates. Reconnect to apply.';

export function displayUpdateAge(updatedAt: string | null | undefined, now = Date.now()): string | null {
  const updated = Date.parse(updatedAt ?? '');
  if (!Number.isFinite(updated)) return null;
  const seconds = Math.max(0, Math.floor((now - updated) / 1000));
  return seconds < 60 ? `${seconds}s ago` : `${Math.floor(seconds / 60)}m ago`;
}

export function idleDisplayLabel(session: SessionInfo, now = Date.now()): string | null {
  if (session.status !== 'connected' || session.displayPaused) return null;
  if (session.idlePreviewState === 'preview') return 'Updating idle preview';
  if (session.idlePreviewState !== 'suppressed') return null;
  const age = displayUpdateAge(session.idlePreviewUpdatedAt, now);
  return age ? `Idle preview · ${age}` : 'Idle preview';
}

/** Fire once when the session view is attached or the app regains focus.
 * A window focus event and a visibility event can describe the same return. */
export function onSessionViewReturn(target: Window, doc: Document, wake: () => void) {
  let active = false;
  const sync = () => {
    const next = doc.visibilityState === 'visible' && doc.hasFocus();
    if (next && !active) wake();
    active = next;
  };
  const blur = () => { active = false; };
  sync();
  target.addEventListener('focus', sync);
  target.addEventListener('blur', blur);
  doc.addEventListener('visibilitychange', sync);
  return () => {
    target.removeEventListener('focus', sync);
    target.removeEventListener('blur', blur);
    doc.removeEventListener('visibilitychange', sync);
  };
}
