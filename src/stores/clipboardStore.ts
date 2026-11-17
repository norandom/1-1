import { create } from 'zustand';
import type { HostProfile } from '../types';
import type { ClipboardController, ClipboardState } from '../lib/clipboard-controller';

interface ClipboardStore {
  sessionId: string | null;
  direction: HostProfile['clipboardDirection'];
  controller: ClipboardController | null;
  status: ClipboardState;
}
export const useClipboardStore = create<ClipboardStore>(() => ({
  sessionId: null, direction: 'off', controller: null,
  status: { kind: 'idle', message: 'Clipboard unavailable' },
}));
