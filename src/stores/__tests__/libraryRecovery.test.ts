import { vi, describe, it, expect, beforeEach } from 'vitest';

// In-memory localStorage, installed before the store module is imported:
// zustand persist reads the storage once at store creation (same pattern
// as libraryStore.test.ts).
const localStorageStub = vi.hoisted(() => {
  const backing = new Map<string, string>();
  const storage: Storage = {
    getItem: (key: string) => (backing.has(key) ? backing.get(key)! : null),
    setItem: (key: string, value: string) => {
      backing.set(key, String(value));
    },
    removeItem: (key: string) => {
      backing.delete(key);
    },
    clear: () => {
      backing.clear();
    },
    key: (index: number) => Array.from(backing.keys())[index] ?? null,
    get length() {
      return backing.size;
    },
  };
  Object.defineProperty(globalThis, 'localStorage', {
    value: storage,
    configurable: true,
    writable: true,
  });
  return storage;
});

// Mock the tauri module before importing the store (existing convention).
vi.mock('../../lib/tauri', () => ({
  listTree: vi.fn(),
  listConnections: vi.fn().mockResolvedValue([]),
  moveNode: vi.fn().mockResolvedValue(undefined),
  getLibraryStatus: vi.fn(),
  reloadLibraryStore: vi.fn(),
}));

import type { LibraryStoreStatus, LibraryTree } from '../../types';
import { RECOVERY_BLOCKED_MESSAGE } from '../../lib/recovery';
import { EMPTY_LIBRARY_TREE, LIBRARY_TREE_STORAGE_KEY, useLibraryStore } from '../libraryStore';
import * as tauri from '../../lib/tauri';

/**
 * Task 8.7 (Req 10.12): the store recovery flow. The store holds the
 * backend `LibraryStoreStatus` (never persisted), rejects its own
 * mutation while write-blocked, and leaves recovery ONLY through a
 * successful reload — which refetches the tree and so restores the
 * normal (enabled) UI.
 */

const TREE: LibraryTree = {
  libraries: [
    { id: 'lib-1', name: 'Work', createdAt: '2026-11-01T00:00:00Z', updatedAt: '2026-11-01T00:00:00Z' },
  ],
  folders: [],
  connections: [
    { id: 'conn-1', name: 'Jump', host: '10.0.0.5', libraryId: 'lib-1', folderId: null },
  ],
};

const BLOCKED: LibraryStoreStatus = {
  status: 'writeBlocked',
  reason: 'migration failed: unsupported schema version 99',
  instructions: 'Restore libraries.json (or the .backup copy) from disk, then choose Reload Library Store.',
};

/** Seed the store as a write-blocked startup would leave it. */
function seedBlocked(tree: LibraryTree = TREE): void {
  useLibraryStore.setState({ tree, status: BLOCKED });
}

describe('libraryStore recovery state (task 8.7, Req 10.12)', () => {
  beforeEach(() => {
    localStorageStub.clear();
    vi.clearAllMocks();
    vi.mocked(tauri.listConnections).mockResolvedValue([]);
    useLibraryStore.setState({
      tree: EMPTY_LIBRARY_TREE,
      selectedNodeId: null,
      expanded: {},
      filter: '',
      tagsById: {},
      isLoading: false,
      error: null,
      lastMoveError: null,
      status: null,
      isReloading: false,
      recoveryError: null,
    });
  });

  // ── Status wiring ───────────────────────────────────────────────────

  it('refreshStatus stores the backend status (ready and write-blocked)', async () => {
    vi.mocked(tauri.getLibraryStatus).mockResolvedValueOnce({ status: 'ready' });
    await useLibraryStore.getState().refreshStatus();
    expect(useLibraryStore.getState().status).toEqual({ status: 'ready' });

    vi.mocked(tauri.getLibraryStatus).mockResolvedValueOnce(BLOCKED);
    await useLibraryStore.getState().refreshStatus();
    expect(useLibraryStore.getState().status).toEqual(BLOCKED);
  });

  it('refreshStatus keeps the previous status when the read itself fails', async () => {
    seedBlocked();
    vi.mocked(tauri.getLibraryStatus).mockRejectedValueOnce(new Error('ipc failed'));
    await useLibraryStore.getState().refreshStatus();
    expect(useLibraryStore.getState().status).toEqual(BLOCKED);
  });

  it('fetchTree during write-block keeps the last-known tree viewable and records the read error', async () => {
    seedBlocked(TREE);
    vi.mocked(tauri.listTree).mockRejectedValueOnce(new Error('store is write-blocked: migration failed'));

    await useLibraryStore.getState().fetchTree();

    const state = useLibraryStore.getState();
    // Reads stay viewable from the last-known state; nothing was cleared.
    expect(state.tree).toBe(TREE);
    expect(state.error).toContain('write-blocked');
  });

  // ── Mutation gating at the store chokepoint ─────────────────────────

  it('moveNode is rejected before any optimistic update while write-blocked', async () => {
    seedBlocked(TREE);

    await expect(
      useLibraryStore.getState().moveNode(
        { kind: 'connection', id: 'conn-1' },
        { kind: 'library', id: 'lib-1' }
      )
    ).rejects.toThrow(RECOVERY_BLOCKED_MESSAGE);

    // No backend call and no optimistic transform: the visible tree (and
    // the on-disk originals) stay exactly as they were (Req 10.12/10.15).
    expect(tauri.moveNode).not.toHaveBeenCalled();
    const state = useLibraryStore.getState();
    expect(state.tree).toBe(TREE);
    expect(state.lastMoveError).toBe(RECOVERY_BLOCKED_MESSAGE);
  });

  // ── Reload flow ─────────────────────────────────────────────────────

  it('reload on a still-broken store keeps the block and records the error (no tree refetch)', async () => {
    seedBlocked(EMPTY_LIBRARY_TREE);
    vi.mocked(tauri.reloadLibraryStore).mockRejectedValueOnce(new Error('load failed: not valid JSON'));

    const recovered = await useLibraryStore.getState().reload();

    expect(recovered).toBe(false);
    const state = useLibraryStore.getState();
    expect(state.status).toEqual(BLOCKED); // stays blocked
    expect(state.recoveryError).toContain('load failed');
    expect(state.isReloading).toBe(false);
    expect(tauri.listTree).not.toHaveBeenCalled(); // no refetch while blocked
  });

  it('a successful reload lifts the block and restores the UI (tree refetched, move works again)', async () => {
    seedBlocked();
    vi.mocked(tauri.reloadLibraryStore).mockResolvedValueOnce({ status: 'ready' });
    vi.mocked(tauri.listTree).mockResolvedValueOnce(TREE);

    const recovered = await useLibraryStore.getState().reload();

    expect(recovered).toBe(true);
    let state = useLibraryStore.getState();
    expect(state.status).toEqual({ status: 'ready' }); // block lifted
    expect(state.tree).toBe(TREE); // tree restored by the refetch
    expect(state.recoveryError).toBeNull();
    expect(state.isReloading).toBe(false);

    // Controls restore only after the valid reload: mutations pass again.
    await useLibraryStore
      .getState()
      .moveNode({ kind: 'connection', id: 'conn-1' }, { kind: 'library', id: 'lib-1' });
    expect(tauri.moveNode).toHaveBeenCalledExactlyOnceWith('connection', 'conn-1', 'library', 'lib-1');
    state = useLibraryStore.getState();
    expect(state.lastMoveError).toBeNull();
  });

  it('transient recovery state is never persisted to localStorage', () => {
    useLibraryStore.setState({
      status: BLOCKED,
      isReloading: true,
      recoveryError: 'boom',
      selectedNodeId: { kind: 'library', id: 'lib-1' },
      expanded: {},
    });

    const persisted = JSON.parse(localStorageStub.getItem(LIBRARY_TREE_STORAGE_KEY)!);
    // Only the UI state survives a reload (8.2 partialize pins); the
    // recovery status is backend-owned transient state.
    expect(Object.keys(persisted.state).sort()).toEqual(['expanded', 'selectedNodeId']);
  });
});
