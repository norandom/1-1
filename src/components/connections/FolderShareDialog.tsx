import { useEffect, useRef, useState } from 'react';
import type { FolderShareDraft, FolderShareState, ShareMode } from '../../types';
import { cancelFolderShare, confirmFolderShare, getFolderShare, revokeFolderShare, selectShareFolder } from '../../lib/tauri';
import { SHARE_PREVIEW_NOTE, SHARE_TIMING_NOTE, folderShareSummary, isPortableShareName, shareModeLabel } from '../../lib/folder-share';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';

/**
 * Share one local folder with a saved host (rdp-folder-sharing 3.2). The
 * native side proves the server key and picks the folder; this dialog shows
 * host, key, folder, name and access together before anything is saved.
 */
export function FolderShareDialog({ id, name, onClose }: { id: string; name: string; onClose: () => void }) {
  const [state, setState] = useState<FolderShareState | null>(null);
  const [draft, setDraft] = useState<FolderShareDraft | null>(null);
  const [shareName, setShareName] = useState('');
  const [mode, setMode] = useState<ShareMode>('readOnly');
  const [busy, setBusy] = useState<'choosing' | 'saving' | 'stopping' | null>(null);
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

  async function choose() {
    setError(null);
    setBusy('choosing');
    try {
      const chosen = await selectShareFolder(id);
      if (chosen) {
        openDraft.current = chosen.selection;
        setDraft(chosen);
        setShareName(chosen.suggestedShareName);
        setMode('readOnly');
      }
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  }

  async function confirm() {
    if (!draft || !isPortableShareName(shareName)) return;
    setError(null);
    setBusy('saving');
    try {
      setState(await confirmFolderShare(draft.selection, shareName, mode));
      openDraft.current = null;
      setDraft(null);
    } catch (cause) {
      setError(String(cause));
      // The native side consumed the draft; a fresh choice is needed.
      openDraft.current = null;
      setDraft(null);
      await refresh();
    } finally {
      setBusy(null);
    }
  }

  async function stop() {
    setError(null);
    setBusy('stopping');
    try {
      setState(await revokeFolderShare(id));
    } catch (cause) {
      setError(String(cause));
      await refresh();
    } finally {
      setBusy(null);
    }
  }

  const nameValid = isPortableShareName(shareName);
  return (
    <Dialog open onOpenChange={(open) => { if (!open && !busy) onClose(); }}>
      <DialogContent showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>Share a folder with {name}</DialogTitle>
          <DialogDescription>Programs in the remote session can use one local folder as a drive. {SHARE_TIMING_NOTE}</DialogDescription>
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
              <p id="share-name-help" className="text-xs text-muted-foreground">1 to 7 letters, digits, _ or -. The remote session shows the drive under this name.</p>
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
            <p role="status">{folderShareSummary(state)}</p>
            {state?.grant && (
              <dl className="grid gap-1">
                <dt className="text-muted-foreground">Host</dt><dd className="font-mono break-all">{state.grant.endpoint}</dd>
                <dt className="text-muted-foreground">Server key</dt><dd className="font-mono break-all">{state.grant.fingerprint}</dd>
              </dl>
            )}
            {state?.supported && <p className="text-xs text-muted-foreground">{SHARE_PREVIEW_NOTE}</p>}
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
              {/* Stopping also resets an unreadable grant file, as its message says. */}
              {(state?.grant || state?.error) && <Button variant="outline" onClick={() => void stop()} disabled={!!busy}>
                {busy === 'stopping' ? 'Stopping…' : 'Stop sharing'}
              </Button>}
              {state?.supported && <Button onClick={() => void choose()} disabled={!!busy}>
                {busy === 'choosing' ? 'Checking the server…' : state.grant ? 'Choose another folder…' : 'Choose folder…'}
              </Button>}
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
