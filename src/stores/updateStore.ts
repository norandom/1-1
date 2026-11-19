import { version } from '../../package.json';
import { create } from 'zustand';
import { checkForUpdates, installUpdate, type UpdateInfo } from '../lib/tauri';
import { useSessionStore } from './sessionStore';
interface UpdateState {
  currentVersion: string;
  update: UpdateInfo | null;
  phase: 'idle' | 'checking' | 'ready' | 'current' | 'downloading' | 'installing' | 'error';
  error: string | null;
  downloaded: number;
  total: number | null;
  startupChecked: boolean;
  check: (startup?: boolean) => Promise<void>;
  install: () => Promise<void>;
}
export function updateBlockedBySessions(): boolean {
  const { sessions, connectRequest, restartRequest } = useSessionStore.getState();
  return !!connectRequest || !!restartRequest || [...sessions.values()].some(s =>
    s.status === 'connected' || s.status === 'connecting' || s.status === 'reconnecting');
}
/** An update is waiting to be installed (found and not yet started). */
export function updateWaiting(state: Pick<UpdateState, 'update' | 'phase'>): boolean {
  return state.update !== null && (state.phase === 'ready' || state.phase === 'error');
}
/** The notice shown once when the startup check finds a newer version. */
export function updateNoticeMessage(version: string): string {
  return `1-1 ${version} is available. Install it from Settings > Updates when no session is connected.`;
}
function message(error: unknown): string { return error instanceof Error ? error.message : String(error); }
function busy(phase: UpdateState['phase']): boolean { return ['checking', 'downloading', 'installing'].includes(phase); }
export const useUpdateStore = create<UpdateState>((set, get) => ({
  currentVersion: version, update: null, phase: 'idle', error: null, downloaded: 0, total: null,
  startupChecked: false,
  check: async (startup = false) => {
    if (busy(get().phase) || (startup && get().startupChecked)) return;
    set({ phase: 'checking', error: null, startupChecked: true });
    try {
      const result = await checkForUpdates();
      set({ currentVersion: result.currentVersion, update: result.update, phase: result.update ? 'ready' : 'current' });
    } catch (error) { set({ phase: 'error', error: message(error), update: null }); }
  },
  install: async () => {
    if (busy(get().phase) || !get().update) return;
    if (updateBlockedBySessions()) { set({ error: 'Disconnect active sessions before installing an update.' }); return; }
    set({ phase: 'downloading', error: null, downloaded: 0, total: null });
    try {
      await installUpdate(progress => set({ phase: progress.phase, downloaded: progress.downloaded, total: progress.total }));
      set({ phase: 'installing' });
    } catch (error) { set({ phase: 'error', error: message(error) }); }
  },
}));

