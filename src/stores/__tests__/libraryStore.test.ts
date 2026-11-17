import { vi, describe, it, expect, beforeEach } from 'vitest';

// In-memory localStorage, installed before the store module is imported:
// zustand persist reads the storage once at store creation.
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
}));

import type { ConnectionConfig, LibraryTree, NodeId } from '../../types';
import {
  FILTER_STATE_PINS,
  TREE_A11Y_STATE,
  TREE_SAVED_EXPANSION_DEFAULT,
  UX_ACCEPTANCE_TREE,
  UX_CONNECTION_TAGS,
} from '../../lib/ux-tree-states';
import {
  EMPTY_LIBRARY_TREE,
  FILTER_EMPTY_STATE_TEXT,
  LIBRARY_TREE_STORAGE_KEY,
  selectEffectiveExpansion,
  selectFilterResult,
  selectNewConnectionDestination,
  selectVisibleNodes,
  useLibraryStore,
} from '../libraryStore';
import * as tauri from '../../lib/tauri';

const lib = (id: string): NodeId => ({ kind: 'library', id });
const fol = (id: string): NodeId => ({ kind: 'folder', id });
const con = (id: string): NodeId => ({ kind: 'connection', id });
const key = (n: NodeId): string => `${n.kind}:${n.id}`;
const keys = (nodes: NodeId[]): string[] => nodes.map(key);

/** Full connection fixture (ConnectionConfig carries tags; summaries do not). */
function fullConnection(id: string, tags: string[]): ConnectionConfig {
  const summary = UX_ACCEPTANCE_TREE.connections.find((c) => c.id === id)!;
  return {
    id: summary.id,
    name: summary.name,
    host: summary.host,
    port: 3389,
    username: 'admin',
    domain: '',
    colorAccent: null,
    tags,
    displayWidth: null,
    displayHeight: null,
    fontSmoothing: true,
    createdAt: '2026-11-01T00:00:00Z',
    updatedAt: '2026-11-01T00:00:00Z',
    libraryId: summary.libraryId,
    folderId: summary.folderId,
    mock: false,
  };
}

/** Seed the store with the 8.1 acceptance tree + saved expansion. */
function seedAcceptanceTree(expanded: Record<string, boolean> = TREE_SAVED_EXPANSION_DEFAULT): void {
  useLibraryStore.setState({
    tree: UX_ACCEPTANCE_TREE,
    tagsById: UX_CONNECTION_TAGS,
    expanded: { ...expanded },
  });
}

describe('libraryStore (task 8.2)', () => {
  beforeEach(() => {
    localStorageStub.clear();
    vi.clearAllMocks();
    useLibraryStore.setState({
      tree: EMPTY_LIBRARY_TREE,
      selectedNodeId: null,
      expanded: {},
      filter: '',
      tagsById: {},
      isLoading: false,
      error: null,
      lastMoveError: null,
    });
  });

  // ── Tree loading (10.1) ─────────────────────────────────────────────

  it('fetchTree loads the tree and aggregates real tags from listConnections (10.9)', async () => {
    vi.mocked(tauri.listTree).mockResolvedValueOnce(UX_ACCEPTANCE_TREE);
    vi.mocked(tauri.listConnections).mockResolvedValueOnce([
      fullConnection('conn-web-dev', ['web', 'dev']),
      fullConnection('conn-term', ['ssh']),
    ]);

    await useLibraryStore.getState().fetchTree();

    const state = useLibraryStore.getState();
    expect(state.tree).toBe(UX_ACCEPTANCE_TREE);
    expect(state.tagsById).toEqual({
      'conn-web-dev': ['web', 'dev'],
      'conn-term': ['ssh'],
    });
    expect(state.isLoading).toBe(false);
    expect(state.error).toBeNull();
  });

  it('fetchTree keeps the previous tree and records the error when listTree fails', async () => {
    seedAcceptanceTree();
    vi.mocked(tauri.listTree).mockRejectedValueOnce(new Error('store unavailable'));

    await useLibraryStore.getState().fetchTree();

    const state = useLibraryStore.getState();
    expect(state.tree).toBe(UX_ACCEPTANCE_TREE);
    expect(state.isLoading).toBe(false);
    expect(state.error).toContain('store unavailable');
  });

  it('fetchTree prunes selection and expansion entries for removed nodes', async () => {
    useLibraryStore.setState({
      selectedNodeId: con('conn-gone'),
      expanded: { 'folder-gone': true, 'folder-work': true },
    });
    vi.mocked(tauri.listTree).mockResolvedValueOnce(UX_ACCEPTANCE_TREE);

    await useLibraryStore.getState().fetchTree();

    const state = useLibraryStore.getState();
    expect(state.selectedNodeId).toBeNull();
    expect(state.expanded).toEqual({ 'folder-work': true });
  });

  // ── Selection + expansion (10.3) ────────────────────────────────────

  it('selectNode stores the typed selection', () => {
    useLibraryStore.getState().selectNode(fol('folder-vms'));
    expect(useLibraryStore.getState().selectedNodeId).toEqual(fol('folder-vms'));

    useLibraryStore.getState().selectNode(null);
    expect(useLibraryStore.getState().selectedNodeId).toBeNull();
  });

  it('toggleExpanded shows/hides children in place by flipping the saved map (10.3)', () => {
    seedAcceptanceTree();

    useLibraryStore.getState().toggleExpanded('folder-vms');
    expect(useLibraryStore.getState().expanded['folder-vms']).toBe(true);

    useLibraryStore.getState().toggleExpanded('folder-work');
    expect(useLibraryStore.getState().expanded['folder-work']).toBe(false);
    // Untouched entries survive.
    expect(useLibraryStore.getState().expanded['lib-team']).toBe(true);
  });

  it('selectVisibleNodes renders depth-first, folders before connections, honoring saved expansion (10.3)', () => {
    seedAcceptanceTree();
    expect(keys(selectVisibleNodes(useLibraryStore.getState()))).toEqual(
      keys(TREE_A11Y_STATE.map((n) => n.node))
    );
  });

  // ── Filter semantics (10.9), pinned against the 8.1 FILTER_STATE_PINS ──

  it('matches every FILTER_STATE_PINS scenario: matched ∪ ancestors, temporary expansion, exact restore', () => {
    for (const pin of FILTER_STATE_PINS) {
      seedAcceptanceTree(); // savedExpansionBefore === TREE_SAVED_EXPANSION_DEFAULT
      const savedBefore = { ...TREE_SAVED_EXPANSION_DEFAULT };

      useLibraryStore.getState().setFilter(pin.query);

      const filtering = useLibraryStore.getState();
      const result = selectFilterResult(filtering);
      expect(result, pin.scenario).not.toBeNull();

      // Matched items and their full ancestor paths stay visible, in render order.
      expect(keys(result!.matched), `${pin.scenario} matched`).toEqual(keys(pin.matched));
      expect(keys(result!.ancestors), `${pin.scenario} ancestors`).toEqual(keys(pin.requiredAncestors));
      expect(keys(result!.visible), `${pin.scenario} visible`).toEqual(keys(pin.visibleDuringFilter));
      expect(keys(selectVisibleNodes(filtering)), `${pin.scenario} visibleNodes`).toEqual(
        keys(pin.visibleDuringFilter)
      );

      // Temporary expansion reveals matches…
      expect(result!.temporaryExpansion, `${pin.scenario} temporaryExpansion`).toEqual(
        pin.temporaryExpansion
      );
      expect(selectEffectiveExpansion(filtering), `${pin.scenario} effective expansion`).toEqual(
        pin.temporaryExpansion
      );

      // …but NEVER mutates the saved expansion while the filter is active.
      expect(filtering.expanded, `${pin.scenario} saved untouched`).toEqual(savedBefore);

      // Clearing restores the prior expansion exactly.
      useLibraryStore.getState().setFilter('');
      const cleared = useLibraryStore.getState();
      expect(cleared.filter).toBe('');
      expect(cleared.expanded, `${pin.scenario} restored`).toEqual(pin.restoredExpansionAfterClear);
      expect(selectFilterResult(cleared), `${pin.scenario} inactive after clear`).toBeNull();
      expect(selectEffectiveExpansion(cleared)).toEqual(pin.restoredExpansionAfterClear);
    }
  });

  it('exposes the pinned no-match empty-state text', () => {
    expect(FILTER_EMPTY_STATE_TEXT).toBe(FILTER_STATE_PINS[3].emptyStateText);
  });

  it('matches folders and libraries by name too (items by name, 10.9)', () => {
    seedAcceptanceTree();

    useLibraryStore.getState().setFilter('vms');
    let result = selectFilterResult(useLibraryStore.getState())!;
    expect(keys(result.matched)).toEqual([key(fol('folder-vms'))]);
    expect(keys(result.visible)).toEqual([key(lib('lib-personal')), key(fol('folder-work')), key(fol('folder-vms'))]);

    useLibraryStore.getState().setFilter('personal');
    result = selectFilterResult(useLibraryStore.getState())!;
    expect(keys(result.matched)).toEqual([key(lib('lib-personal'))]);
    expect(keys(result.visible)).toEqual([key(lib('lib-personal'))]);
  });

  it('a blank filter is inactive', () => {
    seedAcceptanceTree();
    useLibraryStore.getState().setFilter('   ');
    expect(selectFilterResult(useLibraryStore.getState())).toBeNull();
    expect(useLibraryStore.getState().expanded).toEqual(TREE_SAVED_EXPANSION_DEFAULT);
  });

  it('toggleExpanded never mutates saved expansion while the filter is active', () => {
    seedAcceptanceTree();
    useLibraryStore.getState().setFilter('web');

    useLibraryStore.getState().toggleExpanded('folder-vms');
    useLibraryStore.getState().toggleExpanded('lib-personal');

    expect(useLibraryStore.getState().expanded).toEqual(TREE_SAVED_EXPANSION_DEFAULT);
  });

  // ── localStorage persistence (design.md ConnectionTree, 10.11 boundary) ──

  it('persists only selection and expansion to localStorage', () => {
    seedAcceptanceTree();
    useLibraryStore.getState().selectNode(fol('folder-work'));
    useLibraryStore.getState().toggleExpanded('folder-vms'); // saved false in the fixture default

    const raw = localStorageStub.getItem(LIBRARY_TREE_STORAGE_KEY);
    expect(raw).not.toBeNull();
    const persisted = JSON.parse(raw!) as { state: Record<string, unknown>; version: number };
    // The rail preference belongs to AppSettings.sidebarCollapsed (RAIL_STATE_PINS),
    // and the tree comes from the backend — neither lives in tree storage.
    expect(Object.keys(persisted.state).sort()).toEqual(['expanded', 'selectedNodeId']);
    expect(persisted.state.selectedNodeId).toEqual(fol('folder-work'));
    expect(persisted.state.expanded).toEqual({ ...TREE_SAVED_EXPANSION_DEFAULT, 'folder-vms': true });
  });

  it('rehydrates selection and expansion from localStorage on reload', async () => {
    useLibraryStore.setState({ selectedNodeId: null, expanded: {} });
    localStorageStub.setItem(
      LIBRARY_TREE_STORAGE_KEY,
      JSON.stringify({
        state: { selectedNodeId: lib('lib-team'), expanded: { 'lib-team': true, 'folder-prod': true } },
        version: 0,
      })
    );

    await useLibraryStore.persist.rehydrate();

    const state = useLibraryStore.getState();
    expect(state.selectedNodeId).toEqual(lib('lib-team'));
    expect(state.expanded).toEqual({ 'lib-team': true, 'folder-prod': true });
  });

  it('ignores malformed persisted state instead of corrupting the store', async () => {
    localStorageStub.setItem(
      LIBRARY_TREE_STORAGE_KEY,
      JSON.stringify({ state: { selectedNodeId: 'folder-work', expanded: { oops: 'yes' } }, version: 0 })
    );

    await useLibraryStore.persist.rehydrate();

    const state = useLibraryStore.getState();
    expect(state.selectedNodeId).toBeNull();
    expect(state.expanded).toEqual({});
  });

  // ── Node-aware create destination (1.1/1.2 wiring for ConnectionForm) ──

  it('derives the create destination from the selected node', () => {
    seedAcceptanceTree();

    useLibraryStore.setState({ selectedNodeId: fol('folder-vms') });
    expect(selectNewConnectionDestination(useLibraryStore.getState())).toEqual({
      libraryId: 'lib-personal',
      folderId: 'folder-vms',
      label: 'Personal Library › Work › VMs',
    });

    useLibraryStore.setState({ selectedNodeId: lib('lib-team') });
    expect(selectNewConnectionDestination(useLibraryStore.getState())).toEqual({
      libraryId: 'lib-team',
      folderId: null,
      label: 'Team Library',
    });

    // A selected connection files the new one next to it.
    useLibraryStore.setState({ selectedNodeId: con('conn-jump') });
    expect(selectNewConnectionDestination(useLibraryStore.getState())).toEqual({
      libraryId: 'lib-personal',
      folderId: 'folder-lab',
      label: 'Personal Library › Lab',
    });

    useLibraryStore.setState({ selectedNodeId: con('conn-quick') });
    expect(selectNewConnectionDestination(useLibraryStore.getState())).toEqual({
      libraryId: 'lib-team',
      folderId: null,
      label: 'Team Library',
    });

    useLibraryStore.setState({ selectedNodeId: null });
    expect(selectNewConnectionDestination(useLibraryStore.getState())).toBeNull();
  });

  it('returns null for a selected node that no longer exists in the tree', () => {
    seedAcceptanceTree();
    useLibraryStore.setState({ selectedNodeId: fol('folder-gone') });
    expect(selectNewConnectionDestination(useLibraryStore.getState())).toBeNull();
  });

  // ── moveNode: optimistic + rollback (design contract; UI is 8.4) ────

  it('moveNode applies the optimistic move while the IPC is in flight and keeps it on success', async () => {
    seedAcceptanceTree();
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    vi.mocked(tauri.moveNode).mockReturnValueOnce(gate);

    const pending = useLibraryStore.getState().moveNode(con('conn-jump'), fol('folder-vms'));

    // Optimistic: the connection is already under VMs before the IPC settles.
    const inFlight = useLibraryStore.getState().tree;
    const moved = inFlight.connections.find((c) => c.id === 'conn-jump')!;
    expect(moved.folderId).toBe('folder-vms');
    expect(moved.libraryId).toBe('lib-personal');
    expect(useLibraryStore.getState().lastMoveError).toBeNull();

    release();
    await pending;

    // Success keeps the moved state; the wrapper got the flattened NodeIds.
    expect(useLibraryStore.getState().tree).toBe(inFlight);
    expect(tauri.moveNode).toHaveBeenCalledExactlyOnceWith(
      'connection',
      'conn-jump',
      'folder',
      'folder-vms'
    );
  });

  it('moveNode cross-library moves update the subtree libraryIds optimistically', async () => {
    seedAcceptanceTree();

    await useLibraryStore.getState().moveNode(fol('folder-work'), lib('lib-team'));

    const tree = useLibraryStore.getState().tree;
    const folder = (id: string) => tree.folders.find((f) => f.id === id)!;
    const conn = (id: string) => tree.connections.find((c) => c.id === id)!;
    expect(folder('folder-work')).toMatchObject({ libraryId: 'lib-team', parentId: null });
    expect(folder('folder-vms')).toMatchObject({ libraryId: 'lib-team', parentId: 'folder-work' });
    expect(conn('conn-term')).toMatchObject({ libraryId: 'lib-team', folderId: 'folder-work' });
    expect(conn('conn-web-dev')).toMatchObject({ libraryId: 'lib-team', folderId: 'folder-vms' });
    // Untouched nodes stay put.
    expect(conn('conn-web-prod').libraryId).toBe('lib-team');
    expect(folder('folder-lab').libraryId).toBe('lib-personal');
  });

  it('moveNode restores the exact previous tree and records the error on IPC failure', async () => {
    seedAcceptanceTree();
    const before: LibraryTree = useLibraryStore.getState().tree;
    vi.mocked(tauri.moveNode).mockRejectedValueOnce(
      new Error('Cannot move a folder into its own subtree.')
    );

    await expect(
      useLibraryStore.getState().moveNode(fol('folder-work'), fol('folder-vms'))
    ).rejects.toThrow('Cannot move a folder into its own subtree.');

    const state = useLibraryStore.getState();
    expect(state.tree).toBe(before); // exact snapshot, reference-identical
    expect(state.lastMoveError).toBe('Cannot move a folder into its own subtree.');
  });

  it('moveNode clears a previous move error when the next move succeeds', async () => {
    seedAcceptanceTree();
    vi.mocked(tauri.moveNode).mockRejectedValueOnce(new Error('write failed'));
    await expect(
      useLibraryStore.getState().moveNode(con('conn-jump'), fol('folder-vms'))
    ).rejects.toThrow('write failed');

    await useLibraryStore.getState().moveNode(con('conn-jump'), fol('folder-vms'));

    expect(useLibraryStore.getState().lastMoveError).toBeNull();
  });
});
