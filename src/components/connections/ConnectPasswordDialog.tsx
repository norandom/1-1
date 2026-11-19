import { useState } from 'react';
import { usePasswordPrompt, type PasswordRequest } from '../../stores/passwordPromptStore';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';

/** Asks for a host's password at connect; mounted once for the app. */
export function ConnectPasswordDialog() {
  const request = usePasswordPrompt((s) => s.request);
  // A fresh form per request, so nothing typed for one host shows for another.
  return request ? <PasswordForm key={request.connectionId} request={request} /> : null;
}

function PasswordForm({ request }: { request: PasswordRequest }) {
  const answer = usePasswordPrompt((s) => s.answer);
  const [password, setPassword] = useState('');
  const [save, setSave] = useState(true);
  const offerSave = request.reason === 'missing';
  const who = request.username ? `${request.username} on ${request.name}` : request.name;
  return (
    <Dialog open onOpenChange={(open) => { if (!open) answer(null); }}>
      <DialogContent data-local-input>
        <form className="grid gap-4" onSubmit={(event) => { event.preventDefault(); if (password) answer({ password, save: offerSave && save }); }}>
          <DialogHeader>
            <DialogTitle>Password for {request.name}</DialogTitle>
            <DialogDescription>
              {offerSave
                ? `No password is saved for ${who}. Enter it to connect.`
                : `${request.name} does not save its password. Enter it for this session; 1-1 keeps it only until the session ends.`}
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-2">
            <Label htmlFor="connect-password">Password</Label>
            <Input id="connect-password" type="password" autoFocus autoComplete="current-password"
              value={password} onChange={(event) => setPassword(event.target.value)} />
          </div>
          {offerSave && <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={save} onChange={(event) => setSave(event.target.checked)} />
            Save in the system keychain
          </label>}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => answer(null)}>Cancel</Button>
            <Button type="submit" disabled={!password}>Connect</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
