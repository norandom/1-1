import { useId, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { LibraryBig, Plus } from 'lucide-react';
import { selectIsWriteBlocked, useLibraryStore } from '../../stores/libraryStore';
import { RECOVERY_BLOCKED_MESSAGE } from '../../lib/recovery';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import {
  Dialog, DialogContent, DialogDescription, DialogFooter,
  DialogHeader, DialogTitle, DialogTrigger,
} from '../ui/dialog';

/** Sidebar-level creation also works when the tree is empty or collapsed. */
export function NewLibraryDialog({ collapsed }: { collapsed: boolean }) {
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const inputId = useId();
  const errorId = useId();
  const blocked = useLibraryStore(selectIsWriteBlocked);
  const pending = useLibraryStore((state) => state.isCreatingLibrary);
  const createLibrary = useLibraryStore((state) => state.createLibrary);

  const submit = async () => {
    setError(null);
    try {
      await createLibrary(name);
      setOpen(false);
      navigate('/');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => {
      if (pending) return;
      if (next) { setName(''); setError(null); }
      setOpen(next);
    }}>
      <DialogTrigger asChild>
        <Button variant="ghost" disabled={blocked || pending} aria-label="New Library"
          title={blocked ? RECOVERY_BLOCKED_MESSAGE : 'New Library'}
          className={collapsed ? 'h-10 w-10 self-center' : 'w-full justify-start gap-2'}>
          {collapsed ? <Plus size={18} aria-hidden="true" /> : <LibraryBig size={18} aria-hidden="true" />}
          {!collapsed && <span>New Library</span>}
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-sm" showCloseButton={!pending}>
        <DialogHeader>
          <DialogTitle>New Library</DialogTitle>
          <DialogDescription>Organize folders and connections in a new library.</DialogDescription>
        </DialogHeader>
        <form className="grid gap-4" aria-busy={pending} onSubmit={(event) => {
          event.preventDefault();
          if (!pending && !blocked) void submit();
        }}>
          <div className="grid gap-2">
            <label htmlFor={inputId}>Library name</label>
            <Input id={inputId} autoFocus required value={name} disabled={pending || blocked}
              aria-invalid={error ? true : undefined}
              aria-describedby={error || blocked ? errorId : undefined}
              onChange={(event) => { setName(event.target.value); setError(null); }} />
          </div>
          {(error || blocked) && <p id={errorId} role="alert" className="text-sm text-destructive">
            {blocked ? RECOVERY_BLOCKED_MESSAGE : error}
          </p>}
          <DialogFooter>
            <Button type="button" variant="ghost" disabled={pending} onClick={() => setOpen(false)}>Cancel</Button>
            <Button type="submit" disabled={pending || blocked || !name.trim()}>
              {pending ? 'Creating…' : 'Create Library'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
