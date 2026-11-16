import { create } from 'zustand';
import { createJSONStorage, persist } from 'zustand/middleware';
import type { LibraryTree, NodeId } from '../types';
import * as tauri from '../lib/tauri';

// Task 8.2 (Req 10.1/10.3/10.9/10.11): typed tree state for the Royal
// TS-style sidebar — the design.md `LibraryStore` contract. The store owns
// the tree (from `list_tree`), selection, saved expansion and the inline
// filter; rendering is task 8.3, the Move UI is 8.4. Per-connection status
// badges stay in `sessionStore` (design.md: the tree subscribes to it), so
// nothing here duplicates session state.

/** localStorage key for the persisted tree UI state (design.md ConnectionTree). */
export const LIBRARY_TREE_STORAGE_KEY = 'drift-library-tree-state';

/** Pinned no-match text (FILTER_STATE_PINS.emptyStateText, Req 10.9). */
export const FILTER_EMPTY_STATE_TEXT = 'No matching connections';

export const EMPTY_LIBRARY_TREE: LibraryTree = { libraries: [], folders: [], connections: [] };

/** Where a newly created connection is filed, derived from the selection. */
export interface ConnectionDestination {
  libraryId: string;
  folderId: string | null;
  /** Human-readable path (`Library › Folder › …`) for form/dashboard labels. */
  label: string;
}

/** Result of an active filter (Req 10.9): matched items plus all ancestors. */
export interface FilterResult {
  matched: NodeId[];
  ancestors: NodeId[];
  /** Render order of matched ∪ ancestors (depth-first, folders first). */
  visible: NodeId[];
  /** Expansion that reveals the filtered set — temporary, never saved. */
  temporaryExpansion: Record<string, boolean>;
}

interface LibraryStoreState {
  tree: LibraryTree;
  selectedNodeId: NodeId | null;
  /** Saved (persisted) expansion — the only expansion that survives a filter. */
  expanded: Record<string, boolean>;
  filter: string;
  /** id → tags, aggregated from the real `ConnectionConfig.tags` (10.9). */
  tagsById: Record<string, string[]>;
  isLoading: boolean;
  error: string | null;
  lastMoveError: string | null;

  fetchTree: () => Promise<void>;
  selectNode: (id: NodeId | null) => void;
  toggleExpanded: (id: string) => void;
  setFilter: (filter: string) => void;
  /** Optimistic + rollback; rejects with the backend error (UI is 8.4). */
  moveNode: (node: NodeId, target: NodeId) => Promise<void>;
}

// ── Pure tree helpers (shared with the selectors and tests) ──────────

const nodeKey = (n: NodeId): string => `${n.kind}:${n.id}`;

/** Depth-first order ignoring expansion: folders (recursively) before connections per container. */
function flattenTree(tree: LibraryTree): NodeId[] {
  const out: NodeId[] = [];
  const walkFolder = (folderId: string): void => {
    out.push({ kind: 'folder', id: folderId });
    for (const f of tree.folders) if (f.parentId === folderId) walkFolder(f.id);
    for (const c of tree.connections) if (c.folderId === folderId) out.push({ kind: 'connection', id: c.id });
  };
  for (const l of tree.libraries) {
    out.push({ kind: 'library', id: l.id });
    for (const f of tree.folders) if (f.libraryId === l.id && f.parentId === null) walkFolder(f.id);
    for (const c of tree.connections) if (c.libraryId === l.id && c.folderId === null) out.push({ kind: 'connection', id: c.id });
  }
  return out;
}

/** True when the node exists in the tree (used to prune stale selection). */
function treeHasNode(tree: LibraryTree, node: NodeId): boolean {
  if (node.kind === 'library') return tree.libraries.some((l) => l.id === node.id);
  if (node.kind === 'folder') return tree.folders.some((f) => f.id === node.id);
  return tree.connections.some((c) => c.id === node.id);
}

const isFilterActive = (filter: string): boolean => filter.trim().length > 0;

/**
 * Req 10.9: match items by name, host or tags across all libraries.
 * Libraries/folders match by name; connections match by name, host and
 * tags (tags come from the real `ConnectionConfig.tags` via
 * `listConnections` — `ConnectionSummary` carries none). Visible set is
 * matched items plus ALL their ancestor paths.
 */
function applyFilter(tree: LibraryTree, filter: string, tagsById: Record<string, string[]>): FilterResult | null {
  if (!isFilterActive(filter)) return null;
  const query = filter.trim().toLowerCase();

  const matches = (node: NodeId): boolean => {
    if (node.kind === 'library') {
      return tree.libraries.find((l) => l.id === node.id)?.name.toLowerCase().includes(query) ?? false;
    }
    if (node.kind === 'folder') {
      return tree.folders.find((f) => f.id === node.id)?.name.toLowerCase().includes(query) ?? false;
    }
    const c = tree.connections.find((x) => x.id === node.id);
    if (!c) return false;
    const tags = tagsById[c.id] ?? [];
    return (
      c.name.toLowerCase().includes(query) ||
      c.host.toLowerCase().includes(query) ||
      tags.some((t) => t.toLowerCase().includes(query))
    );
  };

  const fullOrder = flattenTree(tree);
  const matched = fullOrder.filter(matches);
  const matchedKeys = new Set(matched.map(nodeKey));

  // Ancestors of every matched node, so matches stay reachable in place.
  const ancestorKeys = new Set<string>();
  for (const node of matched) {
    if (node.kind === 'connection') {
      const c = tree.connections.find((x) => x.id === node.id)!;
      ancestorKeys.add(nodeKey({ kind: 'library', id: c.libraryId }));
      let parentId = c.folderId;
      while (parentId !== null) {
        const folder = tree.folders.find((f) => f.id === parentId);
        if (!folder) break;
        ancestorKeys.add(nodeKey({ kind: 'folder', id: folder.id }));
        parentId = folder.parentId;
      }
    } else if (node.kind === 'folder') {
      let parentId = tree.folders.find((f) => f.id === node.id)?.parentId ?? null;
      while (parentId !== null) {
        const folder = tree.folders.find((f) => f.id === parentId);
        if (!folder) break;
        ancestorKeys.add(nodeKey({ kind: 'folder', id: folder.id }));
        parentId = folder.parentId;
      }
      const libraryId = tree.folders.find((f) => f.id === node.id)?.libraryId;
      if (libraryId) ancestorKeys.add(nodeKey({ kind: 'library', id: libraryId }));
    }
  }

  const visible = fullOrder.filter((n) => matchedKeys.has(nodeKey(n)) || ancestorKeys.has(nodeKey(n)));
  const temporaryExpansion: Record<string, boolean> = {};
  for (const n of visible) {
    if (ancestorKeys.has(nodeKey(n))) temporaryExpansion[n.id] = true;
  }

  return {
    matched,
    ancestors: visible.filter((n) => ancestorKeys.has(nodeKey(n))),
    visible,
    temporaryExpansion,
  };
}

/**
 * Optimistic `move_node` transform (design.md "Storage transactions"):
 * update the moved node's parent plus every descendant's libraryId so a
 * cross-library subtree move lands in one visible transaction. The
 * backend stays authoritative; failures roll back to the exact snapshot.
 */
function optimisticMove(tree: LibraryTree, node: NodeId, target: NodeId): LibraryTree {
  if (node.kind === 'library') return tree; // libraries are not movable (10.7)

  const targetFolder = target.kind === 'folder' ? tree.folders.find((f) => f.id === target.id) : undefined;
  const targetLibraryId = target.kind === 'library' ? target.id : targetFolder?.libraryId;
  if (!targetLibraryId) return tree; // unknown target — let the backend reject it
  const targetParentId = target.kind === 'folder' ? target.id : null;

  if (node.kind === 'connection') {
    return {
      ...tree,
      connections: tree.connections.map((c) =>
        c.id === node.id ? { ...c, libraryId: targetLibraryId, folderId: targetParentId } : c
      ),
    };
  }

  const subtreeIds = new Set<string>([node.id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const f of tree.folders) {
      if (f.parentId !== null && subtreeIds.has(f.parentId) && !subtreeIds.has(f.id)) {
        subtreeIds.add(f.id);
        grew = true;
      }
    }
  }
  return {
    ...tree,
    folders: tree.folders.map((f) =>
      f.id === node.id
        ? { ...f, libraryId: targetLibraryId, parentId: targetParentId }
        : subtreeIds.has(f.id)
          ? { ...f, libraryId: targetLibraryId }
          : f
    ),
    connections: tree.connections.map((c) =>
      subtreeIds.has(c.folderId ?? '') ? { ...c, libraryId: targetLibraryId } : c
    ),
  };
}

// ── Selectors (pure; usable with `useLibraryStore(s => …)` in 8.3) ───

export function selectFilterResult(state: Pick<LibraryStoreState, 'tree' | 'filter' | 'tagsById'>): FilterResult | null {
  return applyFilter(state.tree, state.filter, state.tagsById);
}

/** Visible nodes in render order: the filtered set while filtering, else the saved expansion. */
export function selectVisibleNodes(state: Pick<LibraryStoreState, 'tree' | 'filter' | 'tagsById' | 'expanded'>): NodeId[] {
  const filtered = applyFilter(state.tree, state.filter, state.tagsById);
  if (filtered) return filtered.visible;

  // Walk honoring the saved expansion (10.3: children show/hide in place).
  const out: NodeId[] = [];
  const walkFolder = (folderId: string): void => {
    out.push({ kind: 'folder', id: folderId });
    if (!state.expanded[folderId]) return;
    for (const f of state.tree.folders) if (f.parentId === folderId) walkFolder(f.id);
    for (const c of state.tree.connections) if (c.folderId === folderId) out.push({ kind: 'connection', id: c.id });
  };
  for (const l of state.tree.libraries) {
    out.push({ kind: 'library', id: l.id });
    if (!state.expanded[l.id]) continue;
    for (const f of state.tree.folders) if (f.libraryId === l.id && f.parentId === null) walkFolder(f.id);
    for (const c of state.tree.connections) if (c.libraryId === l.id && c.folderId === null) out.push({ kind: 'connection', id: c.id });
  }
  return out;
}

/** Expansion the tree must render with: temporary while filtering, saved otherwise. */
export function selectEffectiveExpansion(
  state: Pick<LibraryStoreState, 'tree' | 'filter' | 'tagsById' | 'expanded'>
): Record<string, boolean> {
  return applyFilter(state.tree, state.filter, state.tagsById)?.temporaryExpansion ?? state.expanded;
}

/**
 * Destination for a NEW connection derived from the selection (Req 1.2
 * save target, Req 10.1 libraries as roots): a selected folder files
 * inside it, a selected library at its root, a selected connection next
 * to it. Null when nothing is selected or the node vanished — the caller
 * then lets the backend default (default-library root).
 */
export function selectNewConnectionDestination(
  state: Pick<LibraryStoreState, 'tree' | 'selectedNodeId'>
): ConnectionDestination | null {
  const { tree, selectedNodeId: sel } = state;
  if (!sel) return null;

  const libraryName = (id: string): string => tree.libraries.find((l) => l.id === id)?.name ?? id;
  const folderChain = (folderId: string): string[] => {
    const folder = tree.folders.find((f) => f.id === folderId);
    if (!folder) return [];
    return [...(folder.parentId ? folderChain(folder.parentId) : []), folder.name];
  };

  if (sel.kind === 'library') {
    if (!tree.libraries.some((l) => l.id === sel.id)) return null;
    return { libraryId: sel.id, folderId: null, label: libraryName(sel.id) };
  }
  if (sel.kind === 'folder') {
    const folder = tree.folders.find((f) => f.id === sel.id);
    if (!folder) return null;
    return {
      libraryId: folder.libraryId,
      folderId: folder.id,
      label: [libraryName(folder.libraryId), ...folderChain(folder.id)].join(' › '),
    };
  }
  const conn = tree.connections.find((c) => c.id === sel.id);
  if (!conn) return null;
  const label = conn.folderId
    ? [libraryName(conn.libraryId), ...folderChain(conn.folderId)].join(' › ')
    : libraryName(conn.libraryId);
  return { libraryId: conn.libraryId, folderId: conn.folderId, label };
}

// ── Persisted-state guards (localStorage is untrusted input) ─────────

function isNodeId(value: unknown): value is NodeId {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as { kind?: unknown; id?: unknown };
  return (
    typeof v.id === 'string' &&
    (v.kind === 'library' || v.kind === 'folder' || v.kind === 'connection')
  );
}

function isBooleanRecord(value: unknown): value is Record<string, boolean> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  return Object.values(value).every((v) => typeof v === 'boolean');
}

// ── Store ────────────────────────────────────────────────────────────

export const useLibraryStore = create<LibraryStoreState>()(
  persist(
    (set, get) => ({
      tree: EMPTY_LIBRARY_TREE,
      selectedNodeId: null,
      expanded: {},
      filter: '',
      tagsById: {},
      isLoading: false,
      error: null,
      lastMoveError: null,

      fetchTree: async () => {
        set({ isLoading: true, error: null });
        try {
          // Tags live on the full ConnectionConfig (the tree summary carries
          // none), so aggregate id → tags from the existing read wrapper.
          const [tree, connections] = await Promise.all([tauri.listTree(), tauri.listConnections()]);
          const tagsById: Record<string, string[]> = {};
          for (const c of connections) tagsById[c.id] = c.tags ?? [];

          // Prune persisted selection/expansion entries whose nodes are gone.
          const sel = get().selectedNodeId;
          const expanded: Record<string, boolean> = {};
          for (const l of tree.libraries) if (get().expanded[l.id]) expanded[l.id] = true;
          for (const f of tree.folders) if (get().expanded[f.id]) expanded[f.id] = true;

          set({
            tree,
            tagsById,
            selectedNodeId: sel && treeHasNode(tree, sel) ? sel : null,
            expanded,
            isLoading: false,
          });
        } catch (e) {
          set({ error: String(e), isLoading: false });
        }
      },

      selectNode: (id) => set({ selectedNodeId: id }),

      // Req 10.3: expansion toggles mutate the saved (persisted) map —
      // but never while a filter is active: that expansion is temporary
      // and must restore exactly on clear (FILTER_STATE_PINS).
      toggleExpanded: (id) => {
        if (isFilterActive(get().filter)) return;
        set({ expanded: { ...get().expanded, [id]: !get().expanded[id] } });
      },

      setFilter: (filter) => set({ filter }),

      moveNode: async (node, target) => {
        const previousTree = get().tree;
        set({ tree: optimisticMove(previousTree, node, target), lastMoveError: null });
        try {
          await tauri.moveNode(node.kind, node.id, target.kind, target.id);
        } catch (e) {
          // Roll back to the exact pre-move snapshot (Req 10.15).
          set({ tree: previousTree, lastMoveError: e instanceof Error ? e.message : String(e) });
          throw e;
        }
      },
    }),
    {
      name: LIBRARY_TREE_STORAGE_KEY,
      storage: createJSONStorage(() => localStorage),
      // Only the UI state survives a reload: the tree comes from the
      // backend and the rail preference belongs to AppSettings.
      // sidebarCollapsed (RAIL_STATE_PINS) — never duplicated here.
      partialize: (s) => ({ selectedNodeId: s.selectedNodeId, expanded: s.expanded }),
      merge: (persisted, current) => {
        const p = persisted as Partial<Pick<LibraryStoreState, 'selectedNodeId' | 'expanded'>> | undefined;
        return {
          ...current,
          selectedNodeId: p && isNodeId(p.selectedNodeId) ? p.selectedNodeId : null,
          expanded: p && isBooleanRecord(p.expanded) ? p.expanded : {},
        };
      },
    }
  )
);
