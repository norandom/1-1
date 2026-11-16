import { vi, describe, it, expect, beforeEach } from 'vitest';

// In-memory localStorage, installed before the store module is imported
// (same convention as libraryStore.test.ts: zustand persist reads the
// storage once at store creation).
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

vi.mock('../../lib/tauri', () => ({
  listTree: vi.fn(),
  listConnections: vi.fn().mockResolvedValue([]),
  moveNode: vi.fn().mockResolvedValue(undefined),
}));

import type { NodeId } from '../../types';
import { MOVE_STATE_PINS, UX_ACCEPTANCE_TREE } from '../ux-tree-states';
import { useLibraryStore } from '../../stores/libraryStore';
import * as tauri from '../../lib/tauri';
import {
  MOVE_SUBTREE_MESSAGE,
  dragEventToMove,
  moveDestinations,
  moveTreeNodes,
  validateMove,
} from '../tree-move';

const lib = (id: string): NodeId => ({ kind: 'library', id });
const fol = (id: string): NodeId => ({ kind: 'folder', id });
const con = (id: string): NodeId => ({ kind: 'connection', id });
const key = (n: NodeId): string => `${n.kind}:${n.id}`;

// Fixture shape (UX_ACCEPTANCE_TREE):
//   lib-personal ├─ folder-work ├─ folder-vms ├─ conn-web-dev, conn-db-dev
//                │              └─ conn-term
//                └─ folder-lab ── conn-jump
//   lib-team     ├─ folder-prod ─ conn-web-prod, conn-db-prod, conn-bastion
//                └─ conn-quick (library root)

describe('validateMove: source x target matrix (mirrors backend move_node)', () => {
  it('accepts a same-library folder move into another folder', () => {
    expect(validateMove(UX_ACCEPTANCE_TREE, fol('folder-vms'), fol('folder-lab'))).toEqual({
      ok: true,
      source: fol('folder-vms'),
      target: fol('folder-lab'),
    });
  });

  it('accepts a cross-library folder move (subtree follows, Req 10.8)', () => {
    expect(validateMove(UX_ACCEPTANCE_TREE, fol('folder-work'), fol('folder-prod')).ok).toBe(true);
  });

  it('accepts connection -> folder and connection -> library-root targets', () => {
    expect(validateMove(UX_ACCEPTANCE_TREE, con('conn-jump'), fol('folder-vms')).ok).toBe(true);
    expect(validateMove(UX_ACCEPTANCE_TREE, con('conn-bastion'), lib('lib-personal')).ok).toBe(true);
  });

  it('accepts a folder -> library-root move', () => {
    expect(validateMove(UX_ACCEPTANCE_TREE, fol('folder-vms'), lib('lib-team')).ok).toBe(true);
  });

  it('rejects a library as the move source (libraries are tree roots)', () => {
    const decision = validateMove(UX_ACCEPTANCE_TREE, lib('lib-personal'), fol('folder-lab'));
    expect(decision.ok).toBe(false);
    if (!decision.ok) expect(decision.code).toBe('source-is-library');
  });

  it('rejects a connection as the move target', () => {
    const decision = validateMove(UX_ACCEPTANCE_TREE, con('conn-jump'), con('conn-bastion'));
    expect(decision.ok).toBe(false);
    if (!decision.ok) expect(decision.code).toBe('target-is-connection');
  });

  it('rejects a folder into its own subtree with the pinned message', () => {
    const decision = validateMove(UX_ACCEPTANCE_TREE, fol('folder-work'), fol('folder-vms'));
    expect(decision.ok).toBe(false);
    if (!decision.ok) {
      expect(decision.code).toBe('target-in-source-subtree');
      expect(decision.message).toBe(MOVE_SUBTREE_MESSAGE);
      expect(decision.message).toBe('Cannot move a folder into its own subtree.');
    }
  });

  it('rejects a folder onto itself (degenerate own-subtree case)', () => {
    const decision = validateMove(UX_ACCEPTANCE_TREE, fol('folder-work'), fol('folder-work'));
    expect(decision.ok).toBe(false);
    if (!decision.ok) expect(decision.code).toBe('target-in-source-subtree');
  });

  it('rejects a missing source node', () => {
    const decision = validateMove(UX_ACCEPTANCE_TREE, fol('folder-nope'), fol('folder-lab'));
    expect(decision.ok).toBe(false);
    if (!decision.ok) expect(decision.code).toBe('source-missing');
  });

  it('rejects a missing target folder or library', () => {
    const folderTarget = validateMove(UX_ACCEPTANCE_TREE, con('conn-jump'), fol('folder-nope'));
    const libraryTarget = validateMove(UX_ACCEPTANCE_TREE, con('conn-jump'), lib('lib-nope'));
    expect(folderTarget.ok).toBe(false);
    if (!folderTarget.ok) expect(folderTarget.code).toBe('target-missing');
    expect(libraryTarget.ok).toBe(false);
    if (!libraryTarget.ok) expect(libraryTarget.code).toBe('target-missing');
  });
});

describe('moveDestinations: the Move dialog offers valid targets only', () => {
  it('excludes the actor folder and its whole subtree', () => {
    const dests = moveDestinations(UX_ACCEPTANCE_TREE, fol('folder-work')).map((d) => key(d.node));
    expect(dests).not.toContain('folder:folder-work');
    expect(dests).not.toContain('folder:folder-vms'); // descendant
    expect(dests).toEqual([
      'library:lib-personal',
      'library:lib-team',
      'folder:folder-lab',
      'folder:folder-prod',
    ]);
  });

  it('offers every library and folder for a connection actor', () => {
    const dests = moveDestinations(UX_ACCEPTANCE_TREE, con('conn-jump')).map((d) => key(d.node));
    expect(dests).toEqual([
      'library:lib-personal',
      'library:lib-team',
      'folder:folder-work',
      'folder:folder-vms',
      'folder:folder-lab',
      'folder:folder-prod',
    ]);
  });

  it('offers no destination for a library actor (not movable)', () => {
    expect(moveDestinations(UX_ACCEPTANCE_TREE, lib('lib-personal'))).toEqual([]);
  });

  it('labels library roots and folder chains exactly as the dialog renders them', () => {
    const dests = moveDestinations(UX_ACCEPTANCE_TREE, con('conn-jump'));
    expect(dests.find((d) => d.node.kind === 'library' && d.node.id === 'lib-personal')?.label).toBe(
      'Personal Library (root)'
    );
    expect(dests.find((d) => key(d.node) === 'folder:folder-vms')?.label).toBe(
      'Personal Library › Work › VMs'
    );
  });

  it('is exactly the set validateMove accepts (no duplicated validation can drift)', () => {
    for (const actor of [...UX_ACCEPTANCE_TREE.folders.map((f) => fol(f.id)), ...UX_ACCEPTANCE_TREE.connections.map((c) => con(c.id))]) {
      const offered = new Set(moveDestinations(UX_ACCEPTANCE_TREE, actor).map((d) => key(d.node)));
      const candidates = [
        ...UX_ACCEPTANCE_TREE.libraries.map((l) => lib(l.id)),
        ...UX_ACCEPTANCE_TREE.folders.map((f) => fol(f.id)),
      ];
      for (const candidate of candidates) {
        const isSelf = key(candidate) === key(actor);
        expect(offered.has(key(candidate))).toBe(
          isSelf ? false : validateMove(UX_ACCEPTANCE_TREE, actor, candidate).ok
        );
      }
    }
  });
});

describe('dragEventToMove: drag ids -> (source, target) pairs', () => {
  it('maps a drag-end onto a container to the node pair', () => {
    expect(dragEventToMove({ activeKey: 'connection:conn-bastion', overKey: 'folder:folder-vms' })).toEqual({
      source: con('conn-bastion'),
      target: fol('folder-vms'),
    });
    expect(dragEventToMove({ activeKey: 'folder:folder-lab', overKey: 'library:lib-team' })).toEqual({
      source: fol('folder-lab'),
      target: lib('lib-team'),
    });
  });

  it('returns null when the drag ended over nothing', () => {
    expect(dragEventToMove({ activeKey: 'connection:conn-jump', overKey: null })).toBeNull();
  });

  it('returns null for malformed or unknown-kind keys (never invents a move)', () => {
    expect(dragEventToMove({ activeKey: 'bogus', overKey: 'folder:folder-lab' })).toBeNull();
    expect(dragEventToMove({ activeKey: 'widget:x', overKey: 'folder:folder-lab' })).toBeNull();
    expect(dragEventToMove({ activeKey: 'connection:conn-jump', overKey: 'widget:y' })).toBeNull();
  });

  it('a connection drop target still maps — and the shared validation rejects it', () => {
    const pair = dragEventToMove({ activeKey: 'folder:folder-lab', overKey: 'connection:conn-quick' });
    expect(pair).toEqual({ source: fol('folder-lab'), target: con('conn-quick') });
    const decision = validateMove(UX_ACCEPTANCE_TREE, pair!.source, pair!.target);
    expect(decision.ok).toBe(false);
    if (!decision.ok) expect(decision.code).toBe('target-is-connection');
  });
});

describe('moveTreeNodes: the one shared action (store integration)', () => {
  beforeEach(() => {
    localStorageStub.clear();
    vi.clearAllMocks();
    useLibraryStore.setState({
      tree: UX_ACCEPTANCE_TREE,
      selectedNodeId: null,
      expanded: {},
      filter: '',
      tagsById: {},
      isLoading: false,
      error: null,
      lastMoveError: null,
    });
  });

  const deps = () => {
    const s = useLibraryStore.getState();
    return { tree: s.tree, moveNode: s.moveNode };
  };

  it('persists a successful drag-drop and reports focus-follows-node', async () => {
    const outcome = await moveTreeNodes(deps(), 'drag-drop', con('conn-bastion'), fol('folder-vms'));
    expect(outcome).toEqual({
      status: 'moved',
      trigger: 'drag-drop',
      source: con('conn-bastion'),
      target: fol('folder-vms'),
      locationAfter: 'under-destination',
      focusAfter: con('conn-bastion'),
      persistedTreeUnchanged: false,
      message: null,
    });
    expect(tauri.moveNode).toHaveBeenCalledExactlyOnceWith(
      'connection', 'conn-bastion', 'folder', 'folder-vms'
    );
    const moved = useLibraryStore.getState().tree.connections.find((c) => c.id === 'conn-bastion')!;
    expect(moved.folderId).toBe('folder-vms');
    expect(moved.libraryId).toBe('lib-personal');
  });

  it('moves a subtree cross-library through the shared action (10.8)', async () => {
    const outcome = await moveTreeNodes(deps(), 'keyboard-move-dialog', fol('folder-work'), fol('folder-prod'));
    expect(outcome.status).toBe('moved');
    const tree = useLibraryStore.getState().tree;
    expect(tree.folders.find((f) => f.id === 'folder-work')!.libraryId).toBe('lib-team');
    expect(tree.folders.find((f) => f.id === 'folder-vms')!.libraryId).toBe('lib-team');
    for (const id of ['conn-term', 'conn-web-dev', 'conn-db-dev']) {
      expect(tree.connections.find((c) => c.id === id)!.libraryId).toBe('lib-team');
    }
  });

  it('rejects an invalid move WITHOUT touching the store or the backend', async () => {
    const before = useLibraryStore.getState().tree;
    const outcome = await moveTreeNodes(deps(), 'keyboard-move-dialog', fol('folder-work'), fol('folder-vms'));
    expect(outcome).toEqual({
      status: 'restored',
      trigger: 'keyboard-move-dialog',
      source: fol('folder-work'),
      target: fol('folder-vms'),
      locationAfter: 'previous-location',
      focusAfter: fol('folder-work'),
      persistedTreeUnchanged: true,
      message: MOVE_SUBTREE_MESSAGE,
    });
    expect(tauri.moveNode).not.toHaveBeenCalled();
    expect(useLibraryStore.getState().tree).toBe(before); // reference-exact
  });

  it('restores the exact tree and focus when persistence fails (10.15)', async () => {
    vi.mocked(tauri.moveNode).mockRejectedValueOnce(
      new Error('Move failed: the library file could not be written.')
    );
    const before = useLibraryStore.getState().tree;
    const outcome = await moveTreeNodes(deps(), 'drag-drop', con('conn-jump'), fol('folder-vms'));
    expect(outcome).toEqual({
      status: 'restored',
      trigger: 'drag-drop',
      source: con('conn-jump'),
      target: fol('folder-vms'),
      locationAfter: 'previous-location',
      focusAfter: con('conn-jump'),
      persistedTreeUnchanged: true,
      message: 'Move failed: the library file could not be written.',
    });
    // Reference-exact rollback holds THROUGH the action layer, and the
    // store's lastMoveError surfaces the same message it set.
    expect(useLibraryStore.getState().tree).toBe(before);
    expect(useLibraryStore.getState().lastMoveError).toBe(outcome.message);
  });
});

describe('MOVE_STATE_PINS are executable through the shared action', () => {
  beforeEach(() => {
    localStorageStub.clear();
    vi.clearAllMocks();
    useLibraryStore.setState({
      tree: UX_ACCEPTANCE_TREE,
      selectedNodeId: null,
      expanded: {},
      filter: '',
      tagsById: {},
      isLoading: false,
      error: null,
      lastMoveError: null,
    });
  });

  it('every pin reproduces its pinned observable outcome', async () => {
    for (const pin of MOVE_STATE_PINS) {
      useLibraryStore.setState({ tree: UX_ACCEPTANCE_TREE, lastMoveError: null });
      if (pin.failureReason === 'persistence-failed') {
        vi.mocked(tauri.moveNode).mockRejectedValueOnce(new Error(pin.message!));
      }
      const before = useLibraryStore.getState().tree;
      const outcome = await moveTreeNodes(
        { tree: before, moveNode: useLibraryStore.getState().moveNode },
        pin.trigger,
        pin.actor,
        pin.destination
      );
      expect(outcome.locationAfter).toBe(pin.locationAfter);
      expect(outcome.focusAfter).toEqual(pin.focusAfter);
      expect(outcome.message).toBe(pin.message ?? null);
      const unchanged = useLibraryStore.getState().tree === before;
      expect(unchanged).toBe(pin.persistedTreeUnchanged);
      expect(outcome.status).toBe(pin.result === 'succeeds' ? 'moved' : 'restored');
    }
  });
});
