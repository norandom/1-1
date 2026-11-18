import type { NodeId, NodeKind } from '../types';

/**
 * Task 8.3 (Req 10.7/10.13/10.14): the context-menu model for the
 * connection tree. Item sets are the pinned 10.7 actions plus the
 * keyboard `Move…` entry for movable kinds (10.14; libraries are not
 * movable). The focus state machine pins the Shift+F10 contract from
 * ux-acceptance.md Row 1: opening moves focus into the menu, dismissal
 * or selection returns focus to the invoking tree node.
 */

/** Every context action the tree can dispatch, by menu item label. */
export type ContextMenuAction =
  | 'connect'
  | 'disconnect'
  | 'test-credentials'
  | 'review-server-identity'
  | 'edit'
  | 'duplicate'
  | 'move'
  | 'delete'
  | 'new-connection'
  | 'new-folder'
  | 'rename';

/** Pinned 10.7 item sets + `Move…` for movable kinds (10.14). */
export const CONTEXT_MENU_ITEMS: Record<NodeKind, readonly string[]> = {
  connection: ['Connect', 'Test credentials', 'Review server identity', 'Edit', 'Duplicate', 'Move…', 'Delete'],
  folder: ['New Connection', 'New Folder', 'Rename', 'Move…', 'Delete'],
  library: ['New Connection', 'New Folder', 'Rename', 'Delete'],
};

/**
 * The 10.7 item set for a node kind (fresh array; callers may not mutate the
 * pins). A connection with a live session offers Disconnect in Connect's place.
 */
export function contextMenuItems(kind: NodeKind, options: { sessionActive?: boolean } = {}): string[] {
  const items = [...CONTEXT_MENU_ITEMS[kind]];
  return kind === 'connection' && options.sessionActive
    ? items.map((item) => (item === 'Connect' ? 'Disconnect' : item))
    : items;
}

/** Statuses for which a connection's session can be disconnected. */
export function isSessionActive(status: string | null | undefined): boolean {
  return status === 'connecting' || status === 'connected' || status === 'reconnecting';
}

const ITEM_ACTIONS: Record<string, ContextMenuAction> = {
  'Connect': 'connect',
  'Disconnect': 'disconnect',
  'Test credentials': 'test-credentials',
  'Review server identity': 'review-server-identity',
  'Edit': 'edit',
  'Duplicate': 'duplicate',
  'Move…': 'move',
  'Delete': 'delete',
  'New Connection': 'new-connection',
  'New Folder': 'new-folder',
  'Rename': 'rename',
};

/** Action dispatched by a pinned menu item label. */
export function menuActionFor(item: string): ContextMenuAction {
  const action = ITEM_ACTIONS[item];
  if (!action) throw new Error(`unknown context menu item: ${item}`);
  return action;
}

/** Where keyboard focus must live while the menu is open/closed. */
export interface ContextMenuFocus {
  /** `context-menu` while open, `tree-node` once closed (focus returns). */
  target: 'context-menu' | 'tree-node';
  node: NodeId | null;
}

export interface ContextMenuState {
  open: boolean;
  /** Node that invoked the menu — receives focus back on close. */
  invokedFrom: NodeId | null;
  focus: ContextMenuFocus;
}

/** Initial/resting state: closed, no focus target. */
export function closedContextMenu(): ContextMenuState {
  return { open: false, invokedFrom: null, focus: { target: 'tree-node', node: null } };
}

/**
 * Shift+F10 (or right-click) on a tree node: the menu opens and focus
 * moves into it (Req 10.13; focusAfter `context-menu`).
 */
export function openContextMenu(node: NodeId): ContextMenuState {
  return { open: true, invokedFrom: node, focus: { target: 'context-menu', node } };
}

/**
 * Closing the menu — by dismissal (Escape / outside click) or by
 * selecting an item — returns focus to the invoking tree node
 * (ux-acceptance.md Row 1).
 */
export function closeContextMenu(
  state: ContextMenuState,
  via: 'dismiss' | 'select'
): ContextMenuState {
  if (via !== 'dismiss' && via !== 'select') {
    throw new Error(`unknown context menu close cause: ${String(via)}`);
  }
  return { open: false, invokedFrom: state.invokedFrom, focus: { target: 'tree-node', node: state.invokedFrom } };
}
