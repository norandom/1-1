import { useEffect, useRef, useState } from 'react';
import type { FolderShareDraft, FolderShareState, ShareMode } from '../../types';
import { cancelFolderShare, confirmFolderShare, getFolderShare, revokeFolderShare, selectShareFolder } from '../../lib/tauri';
import { MAX_SHARED_FOLDERS, SHARE_SESSION_NOTE, SHARE_TIMING_NOTE, isPortableShareName, shareModeLabel } from '../../lib/folder-share';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';

/**
 * The local folders a saved host may use as drives (rdp-folder-sharing 3.2).
 * Adding one proves the server key natively and picks the folder; this dialog
 * shows host, key, folder, name and access together before anything is saved.
 * Each folder can be stopped on its own.
 */
export function FolderShareDialog({ id, name, onClose }: { id: string; name: string; onClose: () => void }) {
  const [state, setState] = useState<FolderShareState | null>(null);
  const [draft, setDraft] = useState<FolderShareDraft | null>(null);
  const [shareName, setShareName] = useState('');
  const [mode, setMode] = useState<ShareMode>('readOnly');
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const openDraft = useRef<string | null>(null);

  const refresh = () => getFolderShare(id).then(setState, (cause: unknown) => setError(String(cause)));

  useEffect(() => {
    void getFolderShare(id).then(setState, (cause: unknown) => setError(String(cause)));
    return () => {
      // Drafts also expire natively; release this one now.
      if (openDraft.current) void cancelFolderShare(openDraft.current).catch(() => {});
    };
  }, [id]);

  function discardDraft() {
    if (openDraft.current) void cancelFolderShare(openDraft.current).catch(() => {});
    openDraft.current = null;
    setDraft(null);
  }

  async function run(label: string, action: () => Promise<void>) {
    setError(null);
    setBusy(label);
    try {
      await action();
    } catch (cause) {
      setError(String(cause));
      await refresh();
    } finally {
      setBusy(null);
    }
  }

  const choose = () => run('choosing', async () => {
    const chosen = await selectShareFolder(id);
    if (chosen) {
      openDraft.current = chosen.selection;
      setDraft(chosen);
      setShareName(chosen.suggestedShareName);
      setMode('readOnly');
    }
  });

  const confirm = () => run('saving', async () => {
    if (!draft || !isPortableShareName(shareName)) return;
    // The native side consumes the draft either way.
    openDraft.current = null;
    try {
      setState(await confirmFolderShare(draft.selection, shareName, mode));
    } finally {
      setDraft(null);
    }
  });

  const stop = (folder?: string) => run(folder ? `stopping:${folder}` : 'stopping', async () => {
    setState(await revokeFolderShare(id, folder));
  });

  const folders = state?.folders ?? [];
  const taken = folders.some((f) => f.shareName.toLowerCase() === shareName.toLowerCase());
  const nameValid = isPortableShareName(shareName);
  const full = folders.length >= MAX_SHARED_FOLDERS;
  return (
    <Dialog open onOpenChange={(open) => { if (!open && !busy) onClose(); }}>
      <DialogContent showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>Shared folders on {name}</DialogTitle>
          <DialogDescription>Programs in the remote session can use these local folders as drives. {SHARE_TIMING_NOTE}</DialogDescription>
        </DialogHeader>
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        {draft ? (
          <div className="grid gap-3">
            <dl className="grid gap-1 text-sm">
              <dt className="text-muted-foreground">Host</dt><dd className="font-mono break-all">{draft.endpoint}</dd>
              <dt className="text-muted-foreground">Server key</dt><dd className="font-mono break-all">{draft.fingerprint}</dd>
              <dt className="text-muted-foreground">Folder</dt><dd className="break-all">{draft.folder}</dd>
            </dl>
            <div className="grid gap-1">
              <Label htmlFor="share-name">Share name</Label>
              <Input id="share-name" value={shareName} maxLength={7} aria-invalid={!nameValid} aria-describedby="share-name-help"
                onChange={(event) => setShareName(event.target.value)} />
              <p id="share-name-help" className="text-xs text-muted-foreground">
                1 to 7 letters, digits, _ or -. The remote session shows the drive under this name.
                {taken && ' A folder with this name is already shared; saving replaces it.'}
              </p>
            </div>
            <fieldset className="grid gap-1">
              <legend className="text-sm font-medium">Access</legend>
              {(['readOnly', 'readWrite'] as const).map((value) => (
                <label key={value} className="flex items-start gap-2 text-sm">
                  <input type="radio" name="share-mode" value={value} checked={mode === value} onChange={() => setMode(value)} />
                  <span>
                    {shareModeLabel(value)}
                    {value === 'readWrite' && <span className="block text-xs text-muted-foreground">Remote programs can create, change, rename and delete files in this folder.</span>}
                  </span>
                </label>
              ))}
            </fieldset>
          </div>
        ) : (
          <div className="grid gap-2 text-sm">
            {!state && <p role="status">Checking folder sharing…</p>}
            {state && !state.supported && <p role="status">Folder sharing is available in the Windows build.</p>}
            {state?.error && <p role="status">{state.error}</p>}
            {state?.supported && !state.error && folders.length === 0 && <p role="status">No folder is shared with this host.</p>}
            {folders.length > 0 && (
              <ul className="grid gap-2" aria-label="Shared folders">
                {folders.map((folder) => (
                  <li key={folder.shareName} className="flex items-start justify-between gap-2 rounded border p-2">
                    <span className="min-w-0">
                      <span className="block font-medium">{folder.shareName}</span>
                      <span className="block break-all text-xs text-muted-foreground">{folder.folder}</span>
                      <span className="block text-xs">{shareModeLabel(folder.mode)}</span>
                    </span>
                    <Button variant="outline" size="sm" disabled={!!busy} aria-label={`Stop sharing ${folder.shareName}`}
                      onClick={() => void stop(folder.shareName)}>
                      {busy === `stopping:${folder.shareName}` ? 'Stopping…' : 'Stop sharing'}
                    </Button>
                  </li>
                ))}
              </ul>
            )}
            {folders[0] && (
              <dl className="grid gap-1">
                <dt className="text-muted-foreground">Host</dt><dd className="font-mono break-all">{folders[0].endpoint}</dd>
                <dt className="text-muted-foreground">Server key</dt><dd className="font-mono break-all">{folders[0].fingerprint}</dd>
              </dl>
            )}
            {state?.supported && <p className="text-xs text-muted-foreground">{SHARE_SESSION_NOTE}</p>}
          </div>
        )}
        <DialogFooter>
          {draft ? (
            <>
              <Button variant="outline" onClick={discardDraft} disabled={busy === 'saving'}>Cancel</Button>
              <Button onClick={() => void confirm()} disabled={!nameValid || busy === 'saving'}>
                {busy === 'saving' ? 'Saving…' : 'Share folder'}
              </Button>
            </>
          ) : (
            <>
              <Button variant="outline" onClick={onClose} disabled={!!busy}>Close</Button>
              {/* Resetting is how an unreadable grant file is cleared, as its message says. */}
              {state?.error && <Button variant="outline" onClick={() => void stop()} disabled={!!busy}>
                {busy === 'stopping' ? 'Stopping…' : 'Stop sharing all folders'}
              </Button>}
              {state?.supported && !state.error && <Button onClick={() => void choose()} disabled={!!busy || full}
                title={full ? `A host can share up to ${MAX_SHARED_FOLDERS} folders.` : undefined}>
                {busy === 'choosing' ? 'Checking the server…' : folders.length ? 'Add another folder…' : 'Choose folder…'}
              </Button>}
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
