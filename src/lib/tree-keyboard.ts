import type { LibraryTree, NodeId, NodeKind } from '../types';
import type { FocusAfter, TreeKey, TreeKeyOutcome, TreeKeyRule } from './ux-tree-states';
import { TREE_KEYBOARD_RULES } from './ux-tree-states';
import { contextMenuItems } from './tree-menu';

/**
 * Task 8.3 (Req 10.13/10.14): the tree keyboard controller — the roving
 * focus decision logic, implemented exactly against the pinned
 * `TREE_KEYBOARD_RULES` table and verified by `TREE_KEYBOARD_SCENARIOS`.
 * Pure and node-testable: the render layer translates a DOM keydown into
 * a `TreeKeyInput` and applies the resulting focus/expansion/action.
 */

/**
 * The pinned rule a key applies with, resolved by key, node kind and
 * (for containers) expansion state. `expanded` is null for connections.
 */
export function resolveTreeKeyRule(
  key: TreeKey,
  kind: NodeKind,
  expanded: boolean | null
): TreeKeyRule {
  const rule = TREE_KEYBOARD_RULES.find(
    (r) =>
      r.key === key &&
      r.kind === kind &&
      (r.when === undefined ||
        (kind !== 'connection' &&
          ((r.when === 'expanded' && expanded === true) ||
            (r.when === 'collapsed' && expanded === false))))
  );
  if (!rule) throw new Error(`no tree keyboard rule for ${key} on ${kind}`);
  return rule;
}

/** Depth-first visible order honoring expansion; folders before connections. */
export function visibleTreeOrder(tree: LibraryTree, expanded: Record<string, boolean>): NodeId[] {
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

const sameNode = (a: NodeId, b: NodeId): boolean => a.kind === b.kind && a.id === b.id;

/** Parent container of a node (null for libraries). */
function parentNode(tree: LibraryTree, node: NodeId): NodeId | null {
  if (node.kind === 'library') return null;
  if (node.kind === 'folder') {
    const folder = tree.folders.find((f) => f.id === node.id);
    if (!folder) return null;
    return folder.parentId
      ? { kind: 'folder', id: folder.parentId }
      : { kind: 'library', id: folder.libraryId };
  }
  const conn = tree.connections.find((c) => c.id === node.id);
  if (!conn) return null;
  return conn.folderId
    ? { kind: 'folder', id: conn.folderId }
    : { kind: 'library', id: conn.libraryId };
}

/** First direct child in render order (folders before connections). */
function firstChild(tree: LibraryTree, node: NodeId): NodeId | null {
  if (node.kind === 'connection') return null;
  if (node.kind === 'library') {
    const folder = tree.folders.find((f) => f.libraryId === node.id && f.parentId === null);
    if (folder) return { kind: 'folder', id: folder.id };
    const conn = tree.connections.find((c) => c.libraryId === node.id && c.folderId === null);
    return conn ? { kind: 'connection', id: conn.id } : null;
  }
  const folder = tree.folders.find((f) => f.parentId === node.id);
  if (folder) return { kind: 'folder', id: folder.id };
  const conn = tree.connections.find((c) => c.folderId === node.id);
  return conn ? { kind: 'connection', id: conn.id } : null;
}

export type TreeKeyActionType = 'select-dashboard' | 'connect' | 'open-context-menu';

export interface TreeKeyAction {
  type: TreeKeyActionType;
  node: NodeId;
  /** Pinned menu items for `open-context-menu` (10.7 sets + Move…). */
  items?: string[];
}

export interface TreeKeyInput {
  key: TreeKey;
  focus: NodeId;
  tree: LibraryTree;
  /** Expansion to resolve against (saved, or temporary while filtering). */
  expanded: Record<string, boolean>;
}

export interface TreeKeyResult {
  /** The pinned rule that decided this key. */
  rule: TreeKeyRule;
  outcome: TreeKeyOutcome;
  focusAfter: FocusAfter;
  /** Node holding roving tree focus after the key (always visible). */
  focus: NodeId;
  /** Saved-expansion mutation for expand/collapse outcomes, else null. */
  expansionChange: { nodeId: string; expanded: boolean } | null;
  /** Non-navigation side effect, if any. */
  action: TreeKeyAction | null;
}

/**
 * Apply a tree key: resolves the pinned rule and computes the next
 * focus/expansion/action. Focus moves clamp at the edges and only ever
 * land on visible nodes (collapsed subtrees are skipped by construction).
 */
export function applyTreeKey(input: TreeKeyInput): TreeKeyResult {
  const { key, focus, tree, expanded } = input;
  const isContainer = focus.kind !== 'connection';
  const rule = resolveTreeKeyRule(key, focus.kind, isContainer ? expanded[focus.id] === true : null);

  const base = { rule, outcome: rule.outcome, focusAfter: rule.focusAfter };
  const visible = visibleTreeOrder(tree, expanded);
  const index = visible.findIndex((n) => sameNode(n, focus));
  if (index < 0) {
    // Focused node not visible (e.g. its parent collapsed underneath it):
    // no navigation — the render layer re-anchors focus to a visible node.
    return { ...base, focus, expansionChange: null, action: null };
  }

  switch (rule.outcome) {
    case 'focus-previous-visible-node':
      return { ...base, focus: visible[Math.max(0, index - 1)], expansionChange: null, action: null };
    case 'focus-next-visible-node':
      return {
        ...base,
        focus: visible[Math.min(visible.length - 1, index + 1)],
        expansionChange: null,
        action: null,
      };
    case 'focus-first-visible-node':
      return { ...base, focus: visible[0], expansionChange: null, action: null };
    case 'focus-last-visible-node':
      return { ...base, focus: visible[visible.length - 1], expansionChange: null, action: null };
    case 'expand-focused-node':
      return {
        ...base,
        focus,
        expansionChange: { nodeId: focus.id, expanded: true },
        action: null,
      };
    case 'collapse-focused-node':
      return {
        ...base,
        focus,
        expansionChange: { nodeId: focus.id, expanded: false },
        action: null,
      };
    case 'focus-first-child': {
      const child = firstChild(tree, focus);
      return { ...base, focus: child ?? focus, expansionChange: null, action: null };
    }
    case 'focus-parent': {
      const parent = parentNode(tree, focus);
      return { ...base, focus: parent ?? focus, expansionChange: null, action: null };
    }
    case 'select-dashboard':
      return { ...base, focus, expansionChange: null, action: { type: 'select-dashboard', node: focus } };
    case 'connect':
      return { ...base, focus, expansionChange: null, action: { type: 'connect', node: focus } };
    case 'open-context-menu':
      return {
        ...base,
        focus,
        expansionChange: null,
        action: { type: 'open-context-menu', node: focus, items: contextMenuItems(focus.kind) },
      };
    case 'none':
      return { ...base, focus, expansionChange: null, action: null };
  }
}
