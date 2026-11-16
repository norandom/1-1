import type {
  LibraryTree,
  NodeId,
  NodeKind,
  SessionInfo,
  SessionStatus,
} from '../types';
import { SESSION_STATUS_TEXT, type TreeA11yNode } from './ux-tree-states';

/**
 * Task 8.3 (Req 10.3/10.4/10.14): accessible-tree derivation for the
 * ConnectionTree render layer — names, levels, selection, expansion and
 * a textual status for every visible node, exactly as pinned by
 * `TREE_A11Y_STATE`. Pure functions only; no store or DOM access.
 */

/**
 * Live per-connection statuses from the session store's sessions. One
 * session per connection is the app invariant (Req 11.8/11.9); if several
 * exist the first in iteration order wins, deterministically.
 */
export function statusesFromSessions(
  sessions: Iterable<SessionInfo>
): Record<string, SessionStatus> {
  const out: Record<string, SessionStatus> = {};
  for (const s of sessions) {
    if (!(s.connectionId in out)) out[s.connectionId] = s.status;
  }
  return out;
}

/** Textual per-connection status; absent session = disconnected (design.md). */
export function connectionStatusText(status: SessionStatus | undefined): string {
  return SESSION_STATUS_TEXT[status ?? 'disconnected'];
}

/** Every descendant connection of a container, in tree order (nested included). */
export function descendantConnectionIds(tree: LibraryTree, containerId: string): string[] {
  const isLibrary = tree.libraries.some((l) => l.id === containerId);
  const folderIds = new Set<string>();
  if (isLibrary) {
    // A library's folders all carry its libraryId (moves keep them in sync).
    for (const f of tree.folders) if (f.libraryId === containerId) folderIds.add(f.id);
  } else {
    folderIds.add(containerId);
    let grew = true;
    while (grew) {
      grew = false;
      for (const f of tree.folders) {
        if (f.parentId !== null && folderIds.has(f.parentId) && !folderIds.has(f.id)) {
          folderIds.add(f.id);
          grew = true;
        }
      }
    }
  }
  return tree.connections
    .filter((c) =>
      c.folderId !== null ? folderIds.has(c.folderId) : isLibrary && c.libraryId === containerId
    )
    .map((c) => c.id);
}

/**
 * Aggregate container status `<connected> of <total> connected` over all
 * descendant connections (TREE_AGGREGATE_STATUS_TEXT contract).
 */
export function aggregateStatusText(
  tree: LibraryTree,
  containerId: string,
  statuses: Record<string, SessionStatus>
): string {
  const descendants = descendantConnectionIds(tree, containerId);
  const connected = descendants.filter((id) => statuses[id] === 'connected').length;
  return `${connected} of ${descendants.length} connected`;
}

export interface TreeA11yInput {
  tree: LibraryTree;
  /** Live statuses keyed by connection id (absent = no session). */
  statuses: Record<string, SessionStatus>;
  /** Expansion to render with (saved, or temporary while filtering). */
  expanded: Record<string, boolean>;
  selectedNodeId: NodeId | null;
}

/** Depth-first visible order honoring expansion; folders before connections. */
function visibleOrder(tree: LibraryTree, expanded: Record<string, boolean>): NodeId[] {
  const out: NodeId[] = [];
  const walkFolder = (folderId: string): void => {
    out.push({ kind: 'folder', id: folderId });
    if (!expanded[folderId]) return;
    for (const f of tree.folders) if (f.parentId === folderId) walkFolder(f.id);
    for (const c of tree.connections) if (c.folderId === folderId) out.push({ kind: 'connection', id: c.id });
  };
  for (const l of tree.libraries) {
    out.push({ kind: 'library', id: l.id });
    if (!expanded[l.id]) continue;
    for (const f of tree.folders) if (f.libraryId === l.id && f.parentId === null) walkFolder(f.id);
    for (const c of tree.connections) if (c.libraryId === l.id && c.folderId === null) out.push({ kind: 'connection', id: c.id });
  }
  return out;
}

function folderDepth(tree: LibraryTree, folderId: string): number {
  let depth = 2; // library is level 1, a root folder is level 2
  let current = tree.folders.find((f) => f.id === folderId);
  while (current?.parentId != null) {
    depth += 1;
    current = tree.folders.find((f) => f.id === current!.parentId);
  }
  return depth;
}

function nodeName(tree: LibraryTree, node: NodeId): string {
  if (node.kind === 'library') return tree.libraries.find((l) => l.id === node.id)?.name ?? node.id;
  if (node.kind === 'folder') return tree.folders.find((f) => f.id === node.id)?.name ?? node.id;
  return tree.connections.find((c) => c.id === node.id)?.name ?? node.id;
}

function hasChildren(tree: LibraryTree, kind: NodeKind, id: string): boolean {
  if (kind === 'connection') return false;
  if (kind === 'library') {
    return (
      tree.folders.some((f) => f.libraryId === id && f.parentId === null) ||
      tree.connections.some((c) => c.libraryId === id && c.folderId === null)
    );
  }
  return (
    tree.folders.some((f) => f.parentId === id) ||
    tree.connections.some((c) => c.folderId === id)
  );
}

function nodeLevel(tree: LibraryTree, node: NodeId): number {
  if (node.kind === 'library') return 1;
  if (node.kind === 'folder') return folderDepth(tree, node.id);
  const conn = tree.connections.find((c) => c.id === node.id);
  if (!conn) return 2;
  return conn.folderId ? folderDepth(tree, conn.folderId) + 1 : 2;
}

/**
 * The visible tree with everything a11y must expose (Req 10.14), in
 * render order — the computed twin of the pinned `TREE_A11Y_STATE`.
 */
export function buildTreeA11yNodes(input: TreeA11yInput): TreeA11yNode[] {
  const { tree, statuses, expanded, selectedNodeId } = input;
  return visibleOrder(tree, expanded).map((node) => {
    const container = node.kind !== 'connection';
    return {
      node,
      name: nodeName(tree, node),
      kind: node.kind,
      level: nodeLevel(tree, node),
      expanded: container ? expanded[node.id] === true : null,
      selected:
        selectedNodeId !== null &&
        selectedNodeId.kind === node.kind &&
        selectedNodeId.id === node.id,
      hasChildren: hasChildren(tree, node.kind, node.id),
      statusText:
        node.kind === 'connection'
          ? connectionStatusText(statuses[node.id])
          : aggregateStatusText(tree, node.id, statuses),
    };
  });
}

/** Accessible name "name, status" — status is never color-only (10.14). */
export function treeItemLabel(node: TreeA11yNode): string {
  return `${node.name}, ${node.statusText}`;
}

/** ARIA state a treeitem must expose (role itself is set by the render layer). */
export interface TreeItemAria {
  'aria-level': number;
  'aria-selected': boolean;
  'aria-expanded'?: boolean;
}

export function treeItemAria(node: TreeA11yNode): TreeItemAria {
  const aria: TreeItemAria = {
    'aria-level': node.level,
    'aria-selected': node.selected,
  };
  if (node.kind !== 'connection') aria['aria-expanded'] = node.expanded === true;
  return aria;
}
