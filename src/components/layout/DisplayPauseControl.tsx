import { Pause, Play } from 'lucide-react';
import { useId, useRef, useState } from 'react';
import { Button } from '@/components/ui/button';
import { displayUpdateAge } from '../../lib/idle-preview';
import { pauseDisplay, resumeDisplay } from '../../lib/tauri';
import type { SessionInfo } from '../../types';

export function DisplayPauseControl({ session }: { session?: SessionInfo }) {
  const hintId = useId();
  // A tab switch can leave more than one session command awaiting its actor.
  // Key by connection time as a reconnect can reuse the session ID.
  const pending = useRef(new Set<string>());
  const [messages, setMessages] = useState<ReadonlyMap<string, string | null>>(new Map());
  if (session?.status !== 'connected') return null;
  const { id, displayPaused, displayPauseAvailable, displayPauseUnavailableReason } = session;
  const scope = `${id}/${session.connectedAt ?? ''}`;
  const message = messages.get(scope);
  const busy = message === null;
  const age = displayPaused && displayUpdateAge(session.idlePreviewUpdatedAt);
  const reason = displayPauseAvailable ? null : displayPauseUnavailableReason || 'Host support for pausing is unknown.';
  const hint = message || reason || (displayPaused ? `Last update ${age || 'unknown'}` : null);

  function showMessage(value?: string | null) {
    setMessages(previous => {
      const next = new Map(previous);
      if (value !== undefined) next.set(scope, value);
      else next.delete(scope);
      return next;
    });
  }

  async function toggle() {
    if (!displayPauseAvailable || pending.current.has(scope)) return;
    pending.current.add(scope);
    showMessage(null);
    let errorMessage: string | undefined;
    try {
      await (displayPaused ? resumeDisplay(id) : pauseDisplay(id));
    } catch (error) {
      errorMessage = typeof error === 'string' ? error : 'Could not change the display state.';
    } finally {
      pending.current.delete(scope);
      showMessage(errorMessage);
    }
  }

  return <div className="flex min-w-0 items-center gap-1.5 whitespace-nowrap">
    <Button type="button" variant="ghost" size="xs"
      aria-disabled={!displayPauseAvailable || busy} aria-busy={busy}
      aria-describedby={hint ? hintId : undefined}
      title={hint || 'Request a desktop update about once a minute. Remote input resumes live updates.'}
      onMouseDown={event => {
        const focused = event.currentTarget.ownerDocument.activeElement;
        if (event.button === 0 && focused?.tagName === 'CANVAS'
          && focused.closest('[data-rdp-session]')?.getAttribute('data-rdp-session') === id) {
          event.preventDefault();
        }
      }}
      className="h-5 shrink-0 gap-1 text-[10px] aria-disabled:opacity-60" onClick={() => void toggle()}>
      {displayPaused ? <Play size={11} /> : <Pause size={11} />}
      {displayPaused ? 'Resume live' : 'Pause display'}
    </Button>
    {hint && <span id={hintId} className="truncate text-[var(--color-text-muted)]" title={hint}
      role={message ? 'alert' : undefined}>{hint}</span>}
  </div>;
}
