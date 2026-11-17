import { Clipboard } from 'lucide-react';
import { useClipboardStore } from '../../stores/clipboardStore';
import { useSessionStore } from '../../stores/sessionStore';
import { sendsClipboard, receivesClipboard } from '../../lib/clipboard-controller';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel } from '@/components/ui/dropdown-menu';

export function ClipboardStatus({ onOpenChange }: { onOpenChange?: (open: boolean) => void }) {
  const clipboard = useClipboardStore();
  const active = useSessionStore((state) => state.activeSessionId);
  const status = useSessionStore((state) => active ? state.sessions.get(active)?.status : undefined);
  if (!active || status !== 'connected' || clipboard.sessionId !== active) return null;
  const busy = clipboard.status.kind === 'busy';
  return <DropdownMenu onOpenChange={onOpenChange}>
    <DropdownMenuTrigger asChild>
      <Button variant="ghost" size="xs" className="h-5 shrink-0 gap-1 text-[10px]" aria-label="Resync clipboard" title={clipboard.status.message}>
        <Clipboard size={11} /><span aria-live="polite">Clipboard: {clipboard.direction === 'off' ? 'off' : clipboard.status.kind}</span>
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end" className="w-72" data-local-input onEscapeKeyDown={(event) => event.stopPropagation()}>
      <DropdownMenuLabel>Resync clipboard</DropdownMenuLabel>
      <p className="px-2 py-1 text-xs text-muted-foreground">{clipboard.status.message}</p>
      <DropdownMenuItem disabled={busy || !clipboard.controller || !sendsClipboard(clipboard.direction)} onSelect={() => { void clipboard.controller?.toRemote(); }}>Send local clipboard to remote</DropdownMenuItem>
      <DropdownMenuItem disabled={busy || !clipboard.controller || !receivesClipboard(clipboard.direction)} onSelect={() => { void clipboard.controller?.fromRemote(); }}>Get remote clipboard locally</DropdownMenuItem>
      <p className="px-2 py-1 text-xs text-muted-foreground">Choose a direction to avoid overwriting the other clipboard. Host policy changes require reconnect.</p>
    </DropdownMenuContent>
  </DropdownMenu>;
}
