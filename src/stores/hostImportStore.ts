import { create } from 'zustand';
import { parseHostCommand, type HostImport } from '../lib/host-command';

interface HostImportState {
  /** A parsed `1-1 import …` waiting for the user's confirmation. */
  pending: HostImport | null;
  /** Why the last command line could not be imported. */
  error: string | null;
  /** Take a launch's arguments; anything but `import …` is ignored. */
  offer: (args: readonly string[]) => void;
  dismiss: () => void;
}

export const useHostImport = create<HostImportState>((set) => ({
  pending: null,
  error: null,
  offer: (args) => {
    if (args[0] !== 'import') return;
    const parsed = parseHostCommand(args);
    set(parsed.ok ? { pending: parsed.value, error: null } : { pending: null, error: parsed.error });
  },
  dismiss: () => set({ pending: null, error: null }),
}));
