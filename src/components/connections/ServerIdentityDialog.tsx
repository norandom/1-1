import { useEffect, useRef, useState } from 'react';
import type { ServerIdentityReview } from '../../types';
import { cancelServerIdentity, confirmServerIdentity, reviewServerIdentity } from '../../lib/tauri';
import { useToastStore } from '../../stores/toastStore';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';

export function ServerIdentityDialog({ id, name, onClose }: { id: string; name: string; onClose: () => void }) {
  const [review, setReview] = useState<ServerIdentityReview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const submitted = useRef(false);
  const addToast = useToastStore((s) => s.addToast);

  useEffect(() => {
    let closed = false;
    let selection: string | undefined;
    function discard(token: string) {
      // Drafts also expire natively if the webview closes or IPC is unavailable.
      void cancelServerIdentity(token).catch(() => {});
    }
    // StrictMode can mount and immediately clean up before the dialog opens.
    queueMicrotask(() => {
      if (closed) return;
      void reviewServerIdentity(id).then((candidate) => {
        if (closed) discard(candidate.selection);
        else {
          selection = candidate.selection;
          setReview(candidate);
        }
      }, (cause: unknown) => { if (!closed) setError(String(cause)); });
    });
    return () => {
      closed = true;
      if (selection && !submitted.current) discard(selection);
    };
  }, [id]);

  async function confirm() {
    if (!review || submitted.current) return;
    submitted.current = true;
    setSaving(true);
    try {
      await confirmServerIdentity(review.selection);
      addToast({ message: `Server identity saved for ${review.endpoint}.`, type: 'success' });
      onClose();
    } catch (cause) {
      setSaving(false);
      setError(String(cause));
      setReview(null);
    }
  }

  const changed = review?.previousFingerprint && review.previousFingerprint !== review.observedFingerprint;
  return (
    <Dialog open onOpenChange={(open) => { if (!open && !saving) onClose(); }}>
      <DialogContent showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>Review server identity</DialogTitle>
          <DialogDescription>Compare the SHA-256 public-key fingerprint for {name} with the value from your administrator.</DialogDescription>
        </DialogHeader>
        {error ? <p role="alert">{error}</p> : review ? (
          <div className="grid gap-3">
            {changed && <p role="alert">The server key has changed.</p>}
            <dl className="grid gap-2">
              <dt>Endpoint</dt><dd className="font-mono break-all">{review.endpoint}</dd>
              {changed && <>
                <dt>Previously confirmed fingerprint</dt>
                <dd className="font-mono break-all">{review.previousFingerprint}</dd>
              </>}
              <dt>Observed fingerprint</dt><dd className="font-mono break-all">{review.observedFingerprint}</dd>
            </dl>
          </div>
        ) : <p role="status">Checking the server identity…</p>}
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={saving}>{error ? 'Close' : 'Cancel'}</Button>
          {review && <Button onClick={() => void confirm()} disabled={saving}>
            {saving ? 'Saving…' : changed ? 'Confirm changed key' : 'Confirm fingerprint'}
          </Button>}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
