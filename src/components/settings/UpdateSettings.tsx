import { useUpdateStore, updateBlockedBySessions } from '../../stores/updateStore';
import { useSessionStore } from '../../stores/sessionStore';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';

export function UpdateSettings() {
  const state = useUpdateStore();
  // Subscribe to admission/status changes; the action rechecks immediately too.
  useSessionStore(s => s.sessions);
  useSessionStore(s => s.connectRequest);
  useSessionStore(s => s.restartRequest);
  const blocked = updateBlockedBySessions();
  const busy = ['checking', 'downloading', 'installing'].includes(state.phase);
  const download = state.total ? `${Math.min(100, Math.round(state.downloaded / state.total * 100))}%` : `${Math.round(state.downloaded / 1024)} KB`;
  return <Card>
    <CardHeader><CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Updates</CardTitle></CardHeader>
    <CardContent className="space-y-3">
      <p className="text-sm">1-1 {state.currentVersion || '—'} · Preview channel</p>
      <p className="text-xs text-muted-foreground">Checks automatically when the app starts. Installation restarts the app and requires all sessions to be disconnected.</p>
      <div role="status" aria-live="polite" className="text-sm">
        {state.phase === 'checking' && 'Checking for updates…'}
        {state.phase === 'current' && 'You have the latest preview.'}
        {state.update && !busy && `Version ${state.update.version} is available.`}
        {state.phase === 'downloading' && `Downloading update: ${download}`}
        {state.phase === 'installing' && 'Installing verified update…'}
      </div>
      {state.error && <p role="alert" className="text-sm text-destructive">{state.error}</p>}
      {state.update?.notes && <p className="text-xs text-muted-foreground whitespace-pre-wrap max-h-32 overflow-auto">{state.update.notes}</p>}
      <div className="flex gap-2">
        <Button variant="outline" disabled={busy} onClick={() => void state.check()}>Check for updates</Button>
        {state.update && <Button disabled={busy || blocked} onClick={() => void state.install()}>Install and restart</Button>}
      </div>
      {state.update && blocked && <p className="text-xs text-muted-foreground">Disconnect active sessions to enable installation.</p>}
    </CardContent>
  </Card>;
}
