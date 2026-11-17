import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.hoisted(() => {
  const data = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => data.set(key, value),
    removeItem: (key: string) => data.delete(key),
  } });
});
vi.mock('../../lib/tauri', () => ({
  createLibrary: vi.fn(), listTree: vi.fn(), listConnections: vi.fn(),
}));
// SSR normally uses the store's startup snapshot. Supply its current real
// state to the component selector so the production toast can be inspected.
vi.mock('../toastStore', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../toastStore')>();
  const store = actual.useToastStore;
  return { ...actual, useToastStore: Object.assign(
    <T,>(selector: (state: ReturnType<typeof store.getState>) => T) => selector(store.getState()), store,
  ) };
});
import * as tauri from '../../lib/tauri';
import { EMPTY_LIBRARY_TREE, selectVisibleNodes, useLibraryStore } from '../libraryStore';
import { RECOVERY_BLOCKED_MESSAGE } from '../../lib/recovery';
import { createElement } from 'react';
import { renderToString } from 'react-dom/server';
import { useToastStore } from '../toastStore';
import { useSettingsStore } from '../settingsStore';
import { ToastContainer } from '../../components/Toast';

const library = { id: 'created-root', name: 'Work', createdAt: 'now', updatedAt: 'now' };
const tree = { ...EMPTY_LIBRARY_TREE, libraries: [library] };

beforeEach(() => {
  vi.resetAllMocks();
  useToastStore.setState({ toasts: [] });
  useLibraryStore.setState({ tree: EMPTY_LIBRARY_TREE, selectedNodeId: null, expanded: {},
    filter: 'hidden', status: { status: 'ready' }, isCreatingLibrary: false, error: null });
  vi.mocked(tauri.createLibrary).mockResolvedValue(library);
  vi.mocked(tauri.listTree).mockResolvedValue(tree);
  vi.mocked(tauri.listConnections).mockResolvedValue([]);
});

describe('New Library production transaction', () => {
  it.each([true, false])('renders the saved-but-refresh-failed warning with sidebarCollapsed=%s', async (sidebarCollapsed) => {
    useSettingsStore.setState({ settings: { ...useSettingsStore.getState().settings, sidebarCollapsed } });
    vi.mocked(tauri.listTree).mockRejectedValue(new Error('Read unavailable'));
    await expect(useLibraryStore.getState().createLibrary('Work')).resolves.toEqual(library);
    const warning = useToastStore.getState().toasts[0];
    expect(warning).toMatchObject({ type: 'warning', duration: 0 });
    expect(warning.message).toContain('was created');
    expect(warning.message).toContain('Read unavailable');
    // ToastContainer is mounted outside the Sidebar branches in App. Render
    // the real global surface with the committed notification state for SSR.
    const html = renderToString(createElement(ToastContainer));
    expect(html).toContain('Your library is saved; do not create it again.');
    expect(html).toContain('role="alert"');
    expect(tauri.createLibrary).toHaveBeenCalledOnce();
  });
  it('persists a trimmed name, refreshes, clears filter and selects the visible new root', async () => {
    await expect(useLibraryStore.getState().createLibrary('  Work  ')).resolves.toEqual(library);
    expect(tauri.createLibrary).toHaveBeenCalledWith('Work');
    expect(tauri.listTree).toHaveBeenCalledOnce();
    const state = useLibraryStore.getState();
    expect(state.tree).toEqual(tree);
    expect(state.selectedNodeId).toEqual({ kind: 'library', id: library.id });
    expect(state.filter).toBe('');
    expect(selectVisibleNodes(state)).toContainEqual(state.selectedNodeId);
    expect(state.isCreatingLibrary).toBe(false);
  });

  it('rejects blank names before any write', async () => {
    await expect(useLibraryStore.getState().createLibrary(' \t ')).rejects.toThrow('Enter a library name');
    expect(tauri.createLibrary).not.toHaveBeenCalled();
  });

  it('keeps existing state and propagates persistence failures, allowing retry', async () => {
    vi.mocked(tauri.createLibrary).mockRejectedValueOnce(new Error('Disk full'));
    await expect(useLibraryStore.getState().createLibrary('Work')).rejects.toThrow('Disk full');
    expect(useLibraryStore.getState().tree).toEqual(EMPTY_LIBRARY_TREE);
    expect(useLibraryStore.getState().selectedNodeId).toBeNull();
    expect(useLibraryStore.getState().filter).toBe('hidden');
    expect(useLibraryStore.getState().isCreatingLibrary).toBe(false);
    expect(tauri.listTree).not.toHaveBeenCalled();
    await expect(useLibraryStore.getState().createLibrary('Work')).resolves.toEqual(library);
  });

  it('rejects a duplicate submission while the first write is pending', async () => {
    let resolve!: (value: typeof library) => void;
    vi.mocked(tauri.createLibrary).mockReturnValue(new Promise((done) => { resolve = done; }));
    const first = useLibraryStore.getState().createLibrary('Work');
    expect(useLibraryStore.getState().isCreatingLibrary).toBe(true);
    await expect(useLibraryStore.getState().createLibrary('Work')).rejects.toThrow('already being created');
    expect(tauri.createLibrary).toHaveBeenCalledOnce();
    resolve(library);
    await first;
  });

  it('blocks writes in recovery even if the dialog was opened before recovery', async () => {
    useLibraryStore.setState({ status: { status: 'writeBlocked', reason: 'Invalid data', instructions: 'Restore and reload' } });
    await expect(useLibraryStore.getState().createLibrary('Work')).rejects.toThrow(RECOVERY_BLOCKED_MESSAGE);
    expect(tauri.createLibrary).not.toHaveBeenCalled();
    expect(useLibraryStore.getState().isCreatingLibrary).toBe(false);
  });

  it('retains the committed receipt if the follow-up tree read fails, avoiding a duplicate retry', async () => {
    vi.mocked(tauri.listTree).mockRejectedValue(new Error('Read failed'));
    await expect(useLibraryStore.getState().createLibrary('Work')).resolves.toEqual(library);
    expect(useLibraryStore.getState().tree.libraries).toEqual([library]);
    expect(useLibraryStore.getState().selectedNodeId?.id).toBe(library.id);
    expect(useLibraryStore.getState().error).toContain('Read failed');
  });
});
