import { useState } from 'react';
import { useHostImport } from '../../stores/hostImportStore';
import { useConnectAction } from '../../hooks/useConnectAction';
import { useLibraryStore } from '../../stores/libraryStore';
import { importHost } from '../../lib/host-import';
import type { HostImport } from '../../lib/host-command';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';

const FINGERPRINT = /^SHA256:[A-Za-z0-9+/]{43}$/;

/** Confirms a `1-1 import …` command line before anything is saved. */
export function ImportHostDialog() {
  const pending = useHostImport((s) => s.pending);
  const error = useHostImport((s) => s.error);
  const dismiss = useHostImport((s) => s.dismiss);
  if (error) {
    return (
      <Dialog open onOpenChange={(open) => { if (!open) dismiss(); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Command line not imported</DialogTitle>
            <DialogDescription>{error}</DialogDescription>
          </DialogHeader>
          <DialogFooter><Button onClick={dismiss}>Close</Button></DialogFooter>
        </DialogContent>
      </Dialog>
    );
  }
  return pending ? <ImportForm key={`${pending.input.host}:${pending.input.port}`} pending={pending} onDone={dismiss} /> : null;
}

function ImportForm({ pending, onDone }: { pending: HostImport; onDone: () => void }) {
  const { connectById } = useConnectAction();
  const libraries = useLibraryStore((s) => s.tree.libraries);
  const [fingerprint, setFingerprint] = useState(pending.fingerprint ?? '');
  const [busy, setBusy] = useState<'import' | 'connect' | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const { input } = pending;
  const library = pending.library ?? libraries[0]?.name ?? 'Hosts';
  const newLibrary = !libraries.some((l) => l.name.toLowerCase() === library.toLowerCase());
  const fingerprintValid = fingerprint === '' || FINGERPRINT.test(fingerprint.trim());

  async function run(connect: boolean) {
    setBusy(connect ? 'connect' : 'import');
    setProblem(null);
    try {
      const outcome = await importHost(pending, fingerprint.trim() || null);
      if (outcome.kind === 'mismatch') {
        setProblem(`The host was saved, but the server presented ${outcome.observed}, not the fingerprint in the command. Its key was not confirmed and 1-1 did not connect. Check with the server's administrator, then use Review server identity.`);
        return;
      }
      onDone();
      if (connect) connectById(outcome.host.id);
    } catch (cause) {
      setProblem(String(cause));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Dialog open onOpenChange={(open) => { if (!open && !busy) onDone(); }}>
      <DialogContent data-local-input>
        <DialogHeader>
          <DialogTitle>Import {input.name}?</DialogTitle>
          <DialogDescription>A command line asks to add this host. Nothing is saved until you choose Import.</DialogDescription>
        </DialogHeader>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
          <dt className="text-muted-foreground">Address</dt><dd className="font-mono break-all">{input.host}:{input.port}</dd>
          {input.username && <><dt className="text-muted-foreground">User</dt><dd className="break-all">{input.domain ? `${input.domain}\\` : ''}{input.username}</dd></>}
          <dt className="text-muted-foreground">Library</dt><dd>{library}{newLibrary && ' (new)'}</dd>
        </dl>
        <div className="grid gap-1">
          <Label htmlFor="import-fingerprint">Server fingerprint</Label>
          <Input id="import-fingerprint" value={fingerprint} placeholder="SHA256:… (optional)" aria-invalid={!fingerprintValid}
            onChange={(event) => setFingerprint(event.target.value)} className="font-mono" />
          <p className="text-xs text-muted-foreground">
            {fingerprint
              ? 'Before saving the key as trusted, 1-1 reads the server’s key without credentials and checks it against this fingerprint.'
              : 'Without a fingerprint the server’s identity is not checked. You can paste it here, or use Review server identity later.'}
          </p>
        </div>
        {problem && <p role="alert" className="text-sm text-destructive">{problem}</p>}
        <DialogFooter>
          <Button variant="outline" onClick={onDone} disabled={!!busy}>{problem ? 'Close' : 'Cancel'}</Button>
          {!problem && <>
            <Button variant="outline" onClick={() => void run(false)} disabled={!!busy || !fingerprintValid}>
              {busy === 'import' ? 'Importing…' : 'Import'}
            </Button>
            <Button onClick={() => void run(true)} disabled={!!busy || !fingerprintValid}>
              {busy === 'connect' ? 'Checking…' : 'Import and connect'}
            </Button>
          </>}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
