import { useCallback, useEffect, useState } from 'react';
import type { FolderShareState } from '../../types';
import { getFolderShare } from '../../lib/tauri';
import { SHARE_TIMING_NOTE, folderShareSummary } from '../../lib/folder-share';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { FolderShareDialog } from './FolderShareDialog';

/** The saved host's folder share inside its editor (rdp-folder-sharing 3.2). */
export function FolderShareSummary({ id, name }: { id: string; name: string }) {
  const [state, setState] = useState<FolderShareState | null>(null);
  const [managing, setManaging] = useState(false);
  const refresh = useCallback(() => {
    getFolderShare(id).then(setState, (cause: unknown) => setState({ supported: true, folders: [], error: String(cause) }));
  }, [id]);
  useEffect(refresh, [refresh]);

  return (
    <div className="grid gap-1">
      <div className="flex items-center justify-between gap-2">
        <Label>Folder sharing</Label>
        {state?.supported && <Button type="button" variant="outline" size="sm" onClick={() => setManaging(true)}>Manage…</Button>}
      </div>
      <p className="text-sm" role="status">{folderShareSummary(state)}</p>
      {state?.supported && <p className="text-xs text-muted-foreground">{SHARE_TIMING_NOTE}</p>}
      {managing && <FolderShareDialog id={id} name={name} onClose={() => { setManaging(false); refresh(); }} />}
    </div>
  );
}
