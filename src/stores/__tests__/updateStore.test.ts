import { beforeEach, describe, expect, it, vi } from 'vitest';
vi.mock('../../lib/tauri', () => ({ checkForUpdates: vi.fn(), installUpdate: vi.fn() }));
import * as tauri from '../../lib/tauri';
import { useUpdateStore } from '../updateStore';
import { useSessionStore } from '../sessionStore';
import type { SessionInfo } from '../../types';
const available = { currentVersion: '0.1.0-alpha.3', update: { version: '0.1.0-alpha.4', notes: 'Fixes' } };
beforeEach(() => {
  vi.resetAllMocks();
  useUpdateStore.setState({ currentVersion: '', update: null, phase: 'idle', error: null, downloaded: 0, total: null, startupChecked: false });
  useSessionStore.setState({ sessions: new Map(), connectRequest: null, restartRequest: null });
});
describe('verified updater controller', () => {
  it('checks automatically once and permits a later manual check', async () => {
    vi.mocked(tauri.checkForUpdates).mockResolvedValue(available);
    await useUpdateStore.getState().check(true);
    await useUpdateStore.getState().check(true);
    expect(tauri.checkForUpdates).toHaveBeenCalledTimes(1);
    expect(useUpdateStore.getState().phase).toBe('ready');
    await useUpdateStore.getState().check();
    expect(tauri.checkForUpdates).toHaveBeenCalledTimes(2);
  });
  it('reports current version and retryable check errors', async () => {
    vi.mocked(tauri.checkForUpdates).mockRejectedValueOnce('Offline');
    await useUpdateStore.getState().check();
    expect(useUpdateStore.getState().error).toBe('Offline');
    vi.mocked(tauri.checkForUpdates).mockResolvedValue({ currentVersion: '0.1.0-alpha.3', update: null });
    await useUpdateStore.getState().check();
    expect(useUpdateStore.getState().phase).toBe('current');
    expect(useUpdateStore.getState().currentVersion).toBe('0.1.0-alpha.3');
  });
  it.each(['connecting', 'connected', 'reconnecting'] as const)('blocks installation while %s', async status => {
    useUpdateStore.setState({ update: available.update, phase: 'ready' });
    useSessionStore.setState({ sessions: new Map([['active', { status } as SessionInfo]]) });
    await useUpdateStore.getState().install();
    expect(tauri.installUpdate).not.toHaveBeenCalled();
    expect(useUpdateStore.getState().error).toContain('Disconnect');
  });
  it('handles progress and permits retry after download failure', async () => {
    useUpdateStore.setState({ update: available.update, phase: 'ready' });
    vi.mocked(tauri.installUpdate).mockImplementationOnce(async progress => {
      progress({ downloaded: 50, total: 100, phase: 'downloading' });
      expect(useUpdateStore.getState().downloaded).toBe(50);
      throw new Error('Signature rejected');
    });
    await useUpdateStore.getState().install();
    expect(useUpdateStore.getState().error).toBe('Signature rejected');
    expect(useUpdateStore.getState().update).toEqual(available.update);
    vi.mocked(tauri.installUpdate).mockResolvedValue(undefined);
    await useUpdateStore.getState().install();
    expect(tauri.installUpdate).toHaveBeenCalledTimes(2);
  });
});
