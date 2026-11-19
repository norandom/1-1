import { FolderOpen } from 'lucide-react';
import { useSessionStore } from '../../stores/sessionStore';
import { shareModeLabel, shareStatusDetail, shareStatusLabel } from '../../lib/folder-share';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuLabel } from '@/components/ui/dropdown-menu';

/**
 * The active session's shared folder, as the connection achieved it
 * (rdp-folder-sharing 5.3). Hidden when the host shares no folder.
 */
export function FolderShareStatus({ onOpenChange }: { onOpenChange?: (open: boolean) => void }) {
  const active = useSessionStore((state) => state.activeSessionId);
  const share = useSessionStore((state) => active ? state.sessions.get(active)?.folderShare : undefined);
  if (!share || share.state === 'off') return null;
  const label = shareStatusLabel(share);
  return <DropdownMenu onOpenChange={onOpenChange}>
    <DropdownMenuTrigger asChild>
      <Button variant="ghost" size="xs" className="h-5 shrink-0 gap-1 text-[10px]" aria-label={`Shared folder: ${label}`} title={shareStatusDetail(share)}>
        <FolderOpen size={11} /><span aria-live="polite">Folder: {label}</span>
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end" className="w-72" data-local-input onEscapeKeyDown={(event) => event.stopPropagation()}>
      <DropdownMenuLabel>{share.name ? `Shared folder “${share.name}”` : 'Shared folder'}</DropdownMenuLabel>
      {share.readOnly != null && <p className="px-2 py-0.5 text-xs">{shareModeLabel(share.readOnly ? 'readOnly' : 'readWrite')}</p>}
      <p className="px-2 py-1 text-xs text-muted-foreground">{shareStatusDetail(share)}</p>
      {share.lastFailure && <p className="px-2 py-1 text-xs" role="status">
        Last failure: {share.lastFailure.operation}: {share.lastFailure.description} ({share.lastFailure.status})
      </p>}
    </DropdownMenuContent>
  </DropdownMenu>;
}
