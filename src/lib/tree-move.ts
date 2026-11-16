/**
 * Task 8.4 (Req 10.8/10.14/10.15): the ONE shared tree-move action.
 * Both move triggers — the keyboard Move dialog (10.14) and drag-and-drop
 * (10.8) — funnel through `moveTreeNodes`; validation, destination
 * filtering and the drag-event mapping live here so no trigger can drift
 * from the backend `move_node` contract. Pure and node-testable: the only
 * effects flow through the injected 8.2 `moveNode` (optimistic + exact
 * rollback), and the outcome mirrors the MOVE_STATE_PINS semantics
 * (focus follows the node; a rejected write restores the previous
 * location, focus and tree — 10.15).
 */
import type { LibraryTree, NodeId } from '../types';
import type { MoveStatePin } from './ux-tree-states';

/** The two triggers of the shared transaction (MOVE_STATE_PINS contract). */
export type MoveTrigger = MoveStatePin['trigger'];

/** Rejections mirroring the backend `move_node` validation (apply_move). */
export type MoveRejectionCode =
  | 'source-is-library'
  | 'target-is-connection'
  | 'source-missing'
  | 'target-missing'
  | 'target-in-source-subtree';

export type MoveDecision =
  | { ok: true; source: NodeId; target: NodeId }
  | { ok: false; code: MoveRejectionCode; message: string; source: NodeId; target: NodeId };

/** Pinned subtree-failure message (MOVE_STATE_PINS, Req 10.15). */
export const MOVE_SUBTREE_MESSAGE = 'Cannot move a folder into its own subtree.';

/** One selectable destination in the Move dialog / drop mapping. */
export interface MoveDestination {
  node: NodeId;
  label: string;
}

export interface MoveOutcome {
  /** `moved` = the transaction persisted; `restored` = previous state kept. */
  status: 'moved' | 'restored';
  trigger: MoveTrigger;
  source: NodeId;
  target: NodeId;
  locationAfter: 'under-destination' | 'previous-location';
  /** Focus target the caller must move focus to (follows the node). */
  focusAfter: NodeId;
  /** True when the persisted tree never changed (10.15). */
  persistedTreeUnchanged: boolean;
  /** Error message to surface on `restored`; null on success. */
  message: string | null;
}

export interface MoveDeps {
  tree: LibraryTree;
  /** The 8.2 store action: optimistic transform, IPC, rollback + rethrow. */
  moveNode: (node: NodeId, target: NodeId) => Promise<void>;
}

export function validateMove(tree: LibraryTree, source: NodeId, target: NodeId): MoveDecision {
  // Mirrors `apply_move` (store/library.rs): libraries are roots, only
  // folders/libraries receive moves, both ends must exist, and a folder
  // may never land inside its own (pre-move) subtree — itself included.
  if (source.kind === 'library') {
    return {
      ok: false,
      code: 'source-is-library',
      message: 'Libraries cannot be moved; they are the tree roots.',
      source,
      target,
    };
  }
  if (target.kind === 'connection') {
    return {
      ok: false,
      code: 'target-is-connection',
      message: 'Connections cannot receive a move; choose a folder or library.',
      source,
      target,
    };
  }
  const targetExists =
    target.kind === 'library'
      ? tree.libraries.some((l) => l.id === target.id)
      : tree.folders.some((f) => f.id === target.id);
  if (!targetExists) {
    return {
      ok: false,
      code: 'target-missing',
      message: 'The destination no longer exists.',
      source,
      target,
    };
  }
  const sourceExists =
    source.kind === 'connection'
      ? tree.connections.some((c) => c.id === source.id)
      : tree.folders.some((f) => f.id === source.id);
  if (!sourceExists) {
    return {
      ok: false,
      code: 'source-missing',
      message: 'The item to move no longer exists.',
      source,
      target,
    };
  }
  if (source.kind === 'folder' && target.kind === 'folder' && folderSubtree(tree, source.id).has(target.id)) {
    return { ok: false, code: 'target-in-source-subtree', message: MOVE_SUBTREE_MESSAGE, source, target };
  }
  return { ok: true, source, target };
}

/** The folder plus every descendant, at any depth (pre-move tree). */
function folderSubtree(tree: LibraryTree, folderId: string): Set<string> {
  const ids = new Set<string>([folderId]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const f of tree.folders) {
      if (f.parentId !== null && ids.has(f.parentId) && !ids.has(f.id)) {
        ids.add(f.id);
        grew = true;
      }
    }
  }
  return ids;
}

export function moveDestinations(tree: LibraryTree, actor: NodeId): MoveDestination[] {
  if (actor.kind === 'library') return []; // libraries are not movable
  const excluded = actor.kind === 'folder' ? folderSubtree(tree, actor.id) : null;

  const libraryName = (id: string): string => tree.libraries.find((l) => l.id === id)?.name ?? id;
  const folderChain = (folderId: string): string[] => {
    const folder = tree.folders.find((f) => f.id === folderId);
    if (!folder) return [];
    return [...(folder.parentId ? folderChain(folder.parentId) : []), folder.name];
  };

  const destinations: MoveDestination[] = tree.libraries.map((l) => ({
    node: { kind: 'library', id: l.id },
    label: `${l.name} (root)`,
  }));
  for (const f of tree.folders) {
    if (excluded?.has(f.id)) continue;
    destinations.push({
      node: { kind: 'folder', id: f.id },
      label: [libraryName(f.libraryId), ...folderChain(f.id)].join(' › '),
    });
  }
  return destinations;
}

export async function moveTreeNodes(
  deps: MoveDeps,
  trigger: MoveTrigger,
  source: NodeId,
  target: NodeId
): Promise<MoveOutcome> {
  // One shared transaction for BOTH triggers (10.8 drag + 10.14 dialog).
  // A validation rejection never touches the store or the backend: the
  // previous tree is retained by construction and focus returns to the
  // actor's original position (MOVE_STATE_PINS).
  const decision = validateMove(deps.tree, source, target);
  if (!decision.ok) {
    return {
      status: 'restored',
      trigger,
      source,
      target,
      locationAfter: 'previous-location',
      focusAfter: source,
      persistedTreeUnchanged: true,
      message: decision.message,
    };
  }
  try {
    // The 8.2 store action applies the optimistic transform, persists via
    // IPC and rolls back to the exact previous tree before rethrowing.
    await deps.moveNode(source, target);
    return {
      status: 'moved',
      trigger,
      source,
      target,
      locationAfter: 'under-destination',
      focusAfter: source, // focus follows the node under its destination
      persistedTreeUnchanged: false,
      message: null,
    };
  } catch (e) {
    // Same string the store recorded in `lastMoveError` (8.2 derives both
    // from this rejection); the rollback already restored the tree, so
    // focus returns to the actor sitting in its previous location.
    return {
      status: 'restored',
      trigger,
      source,
      target,
      locationAfter: 'previous-location',
      focusAfter: source,
      persistedTreeUnchanged: true,
      message: e instanceof Error ? e.message : String(e),
    };
  }
}

/** Raw drag-end payload (dnd-kit ids are the tree `kind:id` node keys). */
export interface DragPayload {
  activeKey: string;
  overKey: string | null;
}

/** Parse one `kind:id` node key; null for malformed/unknown keys. */
function parseNodeKey(raw: string): NodeId | null {
  const sep = raw.indexOf(':');
  if (sep <= 0) return null;
  const kind = raw.slice(0, sep);
  const id = raw.slice(sep + 1);
  if (kind !== 'library' && kind !== 'folder' && kind !== 'connection') return null;
  if (!id) return null;
  return { kind, id };
}

export function dragEventToMove(payload: DragPayload): { source: NodeId; target: NodeId } | null {
  if (payload.overKey === null) return null; // dropped over nothing — no move
  const source = parseNodeKey(payload.activeKey);
  const target = parseNodeKey(payload.overKey);
  if (!source || !target) return null; // never invent a move from junk ids
  return { source, target };
}
