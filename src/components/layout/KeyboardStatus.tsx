import { Keyboard } from 'lucide-react';
import { useSessionStore } from '../../stores/sessionStore';
import { keyboardDescription, keyboardLabel } from '../../lib/keyboard-status';

/** The keyboard layout the connected session reported (spec rdp-keyboard-layout 5.3). */
export function KeyboardStatus() {
  const active = useSessionStore((state) => state.activeSessionId);
  const session = useSessionStore((state) => (active ? state.sessions.get(active) : undefined));
  const layout = session?.keyboard;
  if (!session || session.status !== 'connected' || !layout) return null;
  const description = keyboardDescription(layout);
  return (
    <span className="flex h-5 shrink-0 items-center gap-1 px-1 text-[10px]" title={description} aria-label={`Keyboard layout: ${description}`}>
      <Keyboard size={11} aria-hidden="true" />
      {keyboardLabel(layout)}
    </span>
  );
}
