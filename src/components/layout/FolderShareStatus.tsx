import { FolderOpen } from 'lucide-react';
import { useSessionStore } from '../../stores/sessionStore';
import { shareModeLabel, shareStatusDetail, shareStatusLabel, sharesSummary } from '../../lib/folder-share';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuLabel, DropdownMenuSeparator } from '@/components/ui/dropdown-menu';

/**
 * The active session's shared folders, as the connection achieved them
 * (rdp-folder-sharing 5.3). Hidden when the host shares no folder.
 */
export function FolderShareStatus({ onOpenChange }: { onOpenChange?: (open: boolean) => void }) {
  const active = useSessionStore((state) => state.activeSessionId);
  const shares = useSessionStore((state) => active ? state.sessions.get(active)?.folderShares : undefined);
  const shown = (shares ?? []).filter((share) => share.state !== 'off');
  if (shown.length === 0) return null;
  const summary = sharesSummary(shown);
  return <DropdownMenu onOpenChange={onOpenChange}>
    <DropdownMenuTrigger asChild>
      <Button variant="ghost" size="xs" className="h-5 shrink-0 gap-1 text-[10px]" aria-label={`Shared folders: ${summary}`}>
        <FolderOpen size={11} /><span aria-live="polite">{summary}</span>
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end" className="w-80" data-local-input onEscapeKeyDown={(event) => event.stopPropagation()}>
      {shown.map((share, index) => (
        <div key={share.name ?? index}>
          {index > 0 && <DropdownMenuSeparator />}
          <DropdownMenuLabel>
            {share.name ? `“${share.name}”` : 'Shared folder'}: {shareStatusLabel(share)}
          </DropdownMenuLabel>
          {share.readOnly != null && <p className="px-2 py-0.5 text-xs">{shareModeLabel(share.readOnly ? 'readOnly' : 'readWrite')}</p>}
          <p className="px-2 py-1 text-xs text-muted-foreground">{shareStatusDetail(share)}</p>
          {share.lastFailure && <p className="px-2 py-1 text-xs" role="status">
            Last failure: {share.lastFailure.operation}: {share.lastFailure.description} ({share.lastFailure.status})
          </p>}
        </div>
      ))}
    </DropdownMenuContent>
  </DropdownMenu>;
}
