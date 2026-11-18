/**
 * Pinned UX acceptance states for the Royal TS-style sidebar tree — the
 * executable form of `.kiro/specs/windows-clearcodec-royalts-e2e/
 * ux-acceptance.md` (task 8.1).
 *
 * Covers the design.md "UX acceptance matrix" rows **Keyboard tree**,
 * **Move**, **Filter** and **Status/rail** (Req 10.3/10.4/10.7-10.9/
 * 10.11/10.13-10.15). Pure data + type contracts only: no component,
 * store or routing behavior lives here, and nothing imports this module
 * at runtime yet. Tasks 8.2-8.4 build `ConnectionTree`/`libraryStore`
 * against these states; their tests import this module as the shared
 * contract. Keep this file and ux-acceptance.md in exact agreement.
 */
import type { LibraryTree, NodeId, NodeKind, SessionStatus } from '../types';

/** Design.md "UX acceptance matrix" row ids (shared with ux-tab-states.ts). */
export type DesignMatrixRow =
  | 'keyboard-tree'
  | 'move'
  | 'filter'
  | 'dashboard-with-tabs'
  | 'close-tab'
  | 'status-rail';

// ── Annotated fixture tree (LibraryTree twin, Req 10.1-10.4) ──────────
// Two libraries, nested folders, a library-root connection (the 10.10
// migration shape), and every live status incl. "no session yet".

export const UX_ACCEPTANCE_TREE: LibraryTree = {
  libraries: [
    { id: 'lib-personal', name: 'Personal Library', createdAt: '2026-11-01T09:00:00Z', updatedAt: '2026-11-01T09:00:00Z' },
    { id: 'lib-team', name: 'Team Library', createdAt: '2026-11-02T10:00:00Z', updatedAt: '2026-11-02T10:00:00Z' },
  ],
  folders: [
    { id: 'folder-work', libraryId: 'lib-personal', parentId: null, name: 'Work' },
    { id: 'folder-vms', libraryId: 'lib-personal', parentId: 'folder-work', name: 'VMs' },
    { id: 'folder-lab', libraryId: 'lib-personal', parentId: null, name: 'Lab' },
    { id: 'folder-prod', libraryId: 'lib-team', parentId: null, name: 'Production' },
  ],
  connections: [
    { id: 'conn-web-dev', name: 'web-dev-01', host: '10.0.0.11', libraryId: 'lib-personal', folderId: 'folder-vms' },
    { id: 'conn-db-dev', name: 'db-dev-02', host: '10.0.0.12', libraryId: 'lib-personal', folderId: 'folder-vms' },
    { id: 'conn-term', name: 'term-box', host: '10.0.0.13', libraryId: 'lib-personal', folderId: 'folder-work' },
    { id: 'conn-jump', name: 'jump-host', host: '10.0.0.14', libraryId: 'lib-personal', folderId: 'folder-lab' },
    { id: 'conn-web-prod', name: 'web-prod', host: '10.0.1.21', libraryId: 'lib-team', folderId: 'folder-prod' },
    { id: 'conn-db-prod', name: 'db-prod', host: '10.0.1.22', libraryId: 'lib-team', folderId: 'folder-prod' },
    { id: 'conn-bastion', name: 'bastion', host: '10.0.1.23', libraryId: 'lib-team', folderId: 'folder-prod' },
    { id: 'conn-quick', name: 'quick-lab', host: '10.0.1.24', libraryId: 'lib-team', folderId: null },
  ],
};

/**
 * Live session-status overlay keyed by connection id (design.md: the
 * tree subscribes `sessionStore` for per-connection badges). Absent =
 * no session exists.
 */
export const UX_SESSION_STATUSES: Record<string, SessionStatus> = {
  'conn-web-dev': 'connected',
  'conn-db-dev': 'connecting',
  'conn-jump': 'reconnecting',
  'conn-web-prod': 'error',
  'conn-bastion': 'connected',
  // A real stopped session (cancelled reconnect / closed elsewhere) —
  // distinct from "no session yet" below, same tree text (design.md).
  'conn-db-prod': 'disconnected',
};

/**
 * Tag overlay: `ConnectionSummary` carries no tags, but the 10.9 filter
 * matches on them, so the pinned filter states need the mapping. Task
 * 8.2 decides how the real store exposes tags; the contract stays.
 */
export const UX_CONNECTION_TAGS: Record<string, string[]> = {
  'conn-web-dev': ['web', 'dev'],
  'conn-db-dev': ['db', 'dev'],
  'conn-term': ['ssh', 'dev'],
  'conn-jump': ['lab', 'jump'],
  'conn-web-prod': ['web', 'prod'],
  'conn-db-prod': ['db', 'prod'],
  'conn-bastion': ['prod'],
  'conn-quick': ['lab'],
};

/**
 * Textual status for every `SessionStatus` — status is never
 * color-only (Req 10.4/10.14). A connection with no session shows the
 * `disconnected` text (design.md: tree status is that single session's
 * status, or disconnected if no session exists).
 */
export const SESSION_STATUS_TEXT: Record<SessionStatus, string> = {
  disconnected: 'Disconnected',
  connecting: 'Connecting…',
  connected: 'Connected',
  reconnecting: 'Reconnecting…',
  error: 'Error',
};

/** Textual aggregate for container nodes (libraries/folders): `<connected> of <total> connected`, counted over all descendant connections. */
export const TREE_AGGREGATE_STATUS_TEXT: Record<string, string> = {
  'lib-personal': '1 of 4 connected',
  'folder-work': '1 of 3 connected',
  'folder-vms': '1 of 2 connected',
  'folder-lab': '0 of 1 connected',
  'lib-team': '1 of 4 connected',
  'folder-prod': '1 of 3 connected',
};

// ── Pinned visible-tree state (Req 10.3/10.14) ────────────────────────

/** Saved (persisted) expansion the pinned states start from. */
export const TREE_SAVED_EXPANSION_DEFAULT: Record<string, boolean> = {
  'lib-personal': true,
  'folder-work': true,
  'folder-vms': false,
  'folder-lab': true,
  'lib-team': true,
  'folder-prod': true,
};

/**
 * One visible tree node with everything a11y must expose (Req 10.14):
 * name, level, kind, expansion, selection and a non-color textual
 * status. Render order (depth-first, folders before connections per
 * container) is the array order.
 */
export interface TreeA11yNode {
  node: NodeId;
  name: string;
  kind: NodeKind;
  /** 1 = library, +1 per folder level. */
  level: number;
  /** Libraries/folders only; `null` for connections. */
  expanded: boolean | null;
  selected: boolean;
  hasChildren: boolean;
  statusText: string;
}

export const TREE_A11Y_STATE: TreeA11yNode[] = [
  { node: { kind: 'library', id: 'lib-personal' }, name: 'Personal Library', kind: 'library', level: 1, expanded: true, selected: false, hasChildren: true, statusText: '1 of 4 connected' },
  { node: { kind: 'folder', id: 'folder-work' }, name: 'Work', kind: 'folder', level: 2, expanded: true, selected: false, hasChildren: true, statusText: '1 of 3 connected' },
  { node: { kind: 'folder', id: 'folder-vms' }, name: 'VMs', kind: 'folder', level: 3, expanded: false, selected: false, hasChildren: true, statusText: '1 of 2 connected' },
  { node: { kind: 'connection', id: 'conn-term' }, name: 'term-box', kind: 'connection', level: 3, expanded: null, selected: false, hasChildren: false, statusText: 'Disconnected' },
  { node: { kind: 'folder', id: 'folder-lab' }, name: 'Lab', kind: 'folder', level: 2, expanded: true, selected: false, hasChildren: true, statusText: '0 of 1 connected' },
  { node: { kind: 'connection', id: 'conn-jump' }, name: 'jump-host', kind: 'connection', level: 3, expanded: null, selected: false, hasChildren: false, statusText: 'Reconnecting…' },
  { node: { kind: 'library', id: 'lib-team' }, name: 'Team Library', kind: 'library', level: 1, expanded: true, selected: false, hasChildren: true, statusText: '1 of 4 connected' },
  { node: { kind: 'folder', id: 'folder-prod' }, name: 'Production', kind: 'folder', level: 2, expanded: true, selected: false, hasChildren: true, statusText: '1 of 3 connected' },
  { node: { kind: 'connection', id: 'conn-web-prod' }, name: 'web-prod', kind: 'connection', level: 3, expanded: null, selected: false, hasChildren: false, statusText: 'Error' },
  { node: { kind: 'connection', id: 'conn-db-prod' }, name: 'db-prod', kind: 'connection', level: 3, expanded: null, selected: false, hasChildren: false, statusText: 'Disconnected' },
  { node: { kind: 'connection', id: 'conn-bastion' }, name: 'bastion', kind: 'connection', level: 3, expanded: null, selected: true, hasChildren: false, statusText: 'Connected' },
  { node: { kind: 'connection', id: 'conn-quick' }, name: 'quick-lab', kind: 'connection', level: 2, expanded: null, selected: false, hasChildren: false, statusText: 'Disconnected' },
];

// ── Keyboard contract (matrix row "Keyboard tree", Req 10.13/10.14) ───

export type TreeKey =
  | 'ArrowUp'
  | 'ArrowDown'
  | 'ArrowLeft'
  | 'ArrowRight'
  | 'Home'
  | 'End'
  | 'Enter'
  | 'Shift+F10';

export type TreeKeyOutcome =
  | 'focus-previous-visible-node'
  | 'focus-next-visible-node'
  | 'focus-first-visible-node'
  | 'focus-last-visible-node'
  | 'expand-focused-node'
  | 'collapse-focused-node'
  | 'focus-first-child'
  | 'focus-parent'
  | 'select-dashboard'
  | 'connect'
  | 'open-context-menu'
  | 'none';

/** Where keyboard focus lands after the key — always visible (Req 10.13). */
export type FocusAfter = 'tree-node' | 'context-menu' | 'opened-session-tab';

/**
 * One row of the roving-focus decision table. Roving tabindex: exactly
 * one tree node is tabbable; arrow/Home/End keys move it, Enter and
 * Shift+F10 act on the focused node. Container keys are split by
 * expansion state (`when`).
 */
export interface TreeKeyRule {
  key: TreeKey;
  kind: NodeKind;
  /** Expansion the rule applies to; omitted for connections. */
  when?: 'expanded' | 'collapsed';
  outcome: TreeKeyOutcome;
  focusAfter: FocusAfter;
}

export const TREE_KEYBOARD_RULES: readonly TreeKeyRule[] = [
  { key: 'ArrowUp', kind: 'library', outcome: 'focus-previous-visible-node', focusAfter: 'tree-node' },
  { key: 'ArrowUp', kind: 'folder', outcome: 'focus-previous-visible-node', focusAfter: 'tree-node' },
  { key: 'ArrowUp', kind: 'connection', outcome: 'focus-previous-visible-node', focusAfter: 'tree-node' },
  { key: 'ArrowDown', kind: 'library', outcome: 'focus-next-visible-node', focusAfter: 'tree-node' },
  { key: 'ArrowDown', kind: 'folder', outcome: 'focus-next-visible-node', focusAfter: 'tree-node' },
  { key: 'ArrowDown', kind: 'connection', outcome: 'focus-next-visible-node', focusAfter: 'tree-node' },
  { key: 'ArrowRight', kind: 'library', when: 'collapsed', outcome: 'expand-focused-node', focusAfter: 'tree-node' },
  { key: 'ArrowRight', kind: 'library', when: 'expanded', outcome: 'focus-first-child', focusAfter: 'tree-node' },
  { key: 'ArrowRight', kind: 'folder', when: 'collapsed', outcome: 'expand-focused-node', focusAfter: 'tree-node' },
  { key: 'ArrowRight', kind: 'folder', when: 'expanded', outcome: 'focus-first-child', focusAfter: 'tree-node' },
  { key: 'ArrowRight', kind: 'connection', outcome: 'none', focusAfter: 'tree-node' },
  { key: 'ArrowLeft', kind: 'library', when: 'expanded', outcome: 'collapse-focused-node', focusAfter: 'tree-node' },
  { key: 'ArrowLeft', kind: 'library', when: 'collapsed', outcome: 'none', focusAfter: 'tree-node' },
  { key: 'ArrowLeft', kind: 'folder', when: 'expanded', outcome: 'collapse-focused-node', focusAfter: 'tree-node' },
  { key: 'ArrowLeft', kind: 'folder', when: 'collapsed', outcome: 'focus-parent', focusAfter: 'tree-node' },
  { key: 'ArrowLeft', kind: 'connection', outcome: 'focus-parent', focusAfter: 'tree-node' },
  { key: 'Home', kind: 'library', outcome: 'focus-first-visible-node', focusAfter: 'tree-node' },
  { key: 'Home', kind: 'folder', outcome: 'focus-first-visible-node', focusAfter: 'tree-node' },
  { key: 'Home', kind: 'connection', outcome: 'focus-first-visible-node', focusAfter: 'tree-node' },
  { key: 'End', kind: 'library', outcome: 'focus-last-visible-node', focusAfter: 'tree-node' },
  { key: 'End', kind: 'folder', outcome: 'focus-last-visible-node', focusAfter: 'tree-node' },
  { key: 'End', kind: 'connection', outcome: 'focus-last-visible-node', focusAfter: 'tree-node' },
  { key: 'Enter', kind: 'library', outcome: 'select-dashboard', focusAfter: 'tree-node' },
  { key: 'Enter', kind: 'folder', outcome: 'select-dashboard', focusAfter: 'tree-node' },
  { key: 'Enter', kind: 'connection', outcome: 'connect', focusAfter: 'opened-session-tab' },
  { key: 'Shift+F10', kind: 'library', outcome: 'open-context-menu', focusAfter: 'context-menu' },
  { key: 'Shift+F10', kind: 'folder', outcome: 'open-context-menu', focusAfter: 'context-menu' },
  { key: 'Shift+F10', kind: 'connection', outcome: 'open-context-menu', focusAfter: 'context-menu' },
];

/**
 * A concrete key transition on the fixture tree. Focus-move scenarios
 * are verifiable against the visible order derived from
 * `TREE_SAVED_EXPANSION_DEFAULT` plus `preconditionExpansion`.
 */
export interface TreeKeyScenario {
  scenario: string;
  key: TreeKey;
  from: NodeId;
  /** Expansion overrides applied to the saved default before the key. */
  preconditionExpansion?: Record<string, boolean>;
  expectFocus: NodeId;
  /** Container expansion change caused by the key. */
  expectExpansionChange?: { nodeId: string; expanded: boolean };
  /** Enter on a library/folder selects its dashboard (10.5/10.13). */
  expectSelectDashboard?: NodeId;
  /** Enter on a connection connects it (10.6/10.13). */
  expectConnect?: string;
  /** Shift+F10 menu items — the 10.7 set plus the 10.14 Move action for movable kinds. */
  expectContextMenuItems?: string[];
}

export const TREE_KEYBOARD_SCENARIOS: readonly TreeKeyScenario[] = [
  { scenario: 'ArrowDown moves focus to the next visible node', key: 'ArrowDown', from: { kind: 'library', id: 'lib-personal' }, expectFocus: { kind: 'folder', id: 'folder-work' } },
  { scenario: 'ArrowDown skips the children of a collapsed folder', key: 'ArrowDown', from: { kind: 'connection', id: 'conn-term' }, expectFocus: { kind: 'folder', id: 'folder-lab' } },
  { scenario: 'ArrowDown on the last visible node clamps focus in place', key: 'ArrowDown', from: { kind: 'connection', id: 'conn-quick' }, expectFocus: { kind: 'connection', id: 'conn-quick' } },
  { scenario: 'ArrowUp moves focus to the previous visible node', key: 'ArrowUp', from: { kind: 'folder', id: 'folder-work' }, expectFocus: { kind: 'library', id: 'lib-personal' } },
  { scenario: 'ArrowUp on the first visible node clamps focus in place', key: 'ArrowUp', from: { kind: 'library', id: 'lib-personal' }, expectFocus: { kind: 'library', id: 'lib-personal' } },
  { scenario: 'ArrowRight expands a collapsed folder and keeps focus on it', key: 'ArrowRight', from: { kind: 'folder', id: 'folder-vms' }, expectFocus: { kind: 'folder', id: 'folder-vms' }, expectExpansionChange: { nodeId: 'folder-vms', expanded: true } },
  { scenario: 'ArrowRight on an expanded folder focuses its first child', key: 'ArrowRight', from: { kind: 'folder', id: 'folder-work' }, expectFocus: { kind: 'folder', id: 'folder-vms' } },
  { scenario: 'ArrowRight on a connection does nothing', key: 'ArrowRight', from: { kind: 'connection', id: 'conn-jump' }, expectFocus: { kind: 'connection', id: 'conn-jump' } },
  { scenario: 'ArrowLeft collapses an expanded folder and keeps focus on it', key: 'ArrowLeft', from: { kind: 'folder', id: 'folder-work' }, expectFocus: { kind: 'folder', id: 'folder-work' }, expectExpansionChange: { nodeId: 'folder-work', expanded: false } },
  { scenario: 'ArrowLeft on a collapsed folder focuses its parent', key: 'ArrowLeft', from: { kind: 'folder', id: 'folder-vms' }, expectFocus: { kind: 'folder', id: 'folder-work' } },
  { scenario: 'ArrowLeft on a connection focuses its parent folder', key: 'ArrowLeft', from: { kind: 'connection', id: 'conn-jump' }, expectFocus: { kind: 'folder', id: 'folder-lab' } },
  { scenario: 'ArrowLeft on a library-root connection focuses its library', key: 'ArrowLeft', from: { kind: 'connection', id: 'conn-quick' }, expectFocus: { kind: 'library', id: 'lib-team' } },
  { scenario: 'ArrowLeft collapses an expanded top-level library', key: 'ArrowLeft', from: { kind: 'library', id: 'lib-team' }, expectFocus: { kind: 'library', id: 'lib-team' }, expectExpansionChange: { nodeId: 'lib-team', expanded: false } },
  { scenario: 'Home focuses the first visible node', key: 'Home', from: { kind: 'connection', id: 'conn-quick' }, expectFocus: { kind: 'library', id: 'lib-personal' } },
  { scenario: 'End focuses the last visible node', key: 'End', from: { kind: 'library', id: 'lib-personal' }, expectFocus: { kind: 'connection', id: 'conn-quick' } },
  { scenario: 'Enter on a folder selects its dashboard and keeps tree focus', key: 'Enter', from: { kind: 'folder', id: 'folder-prod' }, expectFocus: { kind: 'folder', id: 'folder-prod' }, expectSelectDashboard: { kind: 'folder', id: 'folder-prod' } },
  { scenario: 'Enter on a library selects its dashboard and keeps tree focus', key: 'Enter', from: { kind: 'library', id: 'lib-team' }, expectFocus: { kind: 'library', id: 'lib-team' }, expectSelectDashboard: { kind: 'library', id: 'lib-team' } },
  { scenario: 'Enter on a connection connects it and focuses the opened tab', key: 'Enter', from: { kind: 'connection', id: 'conn-bastion' }, expectFocus: { kind: 'connection', id: 'conn-bastion' }, expectConnect: 'conn-bastion' },
  { scenario: 'Enter on a library-root connection connects it', key: 'Enter', from: { kind: 'connection', id: 'conn-quick' }, expectFocus: { kind: 'connection', id: 'conn-quick' }, expectConnect: 'conn-quick' },
  { scenario: 'Shift+F10 opens the connection context menu', key: 'Shift+F10', from: { kind: 'connection', id: 'conn-web-prod' }, expectFocus: { kind: 'connection', id: 'conn-web-prod' }, expectContextMenuItems: ['Connect', 'Test credentials', 'Review server identity', 'Edit', 'Duplicate', 'Move…', 'Delete'] },
  { scenario: 'Shift+F10 opens the folder context menu', key: 'Shift+F10', from: { kind: 'folder', id: 'folder-prod' }, expectFocus: { kind: 'folder', id: 'folder-prod' }, expectContextMenuItems: ['New Connection', 'New Folder', 'Rename', 'Move…', 'Delete'] },
  { scenario: 'Shift+F10 opens the library context menu (libraries are not movable)', key: 'Shift+F10', from: { kind: 'library', id: 'lib-team' }, expectFocus: { kind: 'library', id: 'lib-team' }, expectContextMenuItems: ['New Connection', 'New Folder', 'Rename', 'Delete'] },
];

// ── Move states (matrix row "Move", Req 10.8/10.14/10.15) ─────────────

/**
 * Keyboard Move dialog and drag-and-drop are two triggers of one shared
 * transaction. Success: the node lands under the destination and focus
 * follows it. Failure: the previous persisted tree is retained, the
 * visible location and focus are restored, and an error message shows.
 */
export interface MoveStatePin {
  scenario: string;
  matrixRow: 'move';
  trigger: 'keyboard-move-dialog' | 'drag-drop';
  actor: NodeId;
  destination: NodeId;
  result: 'succeeds' | 'fails';
  failureReason?: 'destination-inside-actor-subtree' | 'persistence-failed';
  locationAfter: 'under-destination' | 'previous-location';
  /** Focus after the action completes (restored on failure). */
  focusAfter: NodeId;
  /** Failure keeps the previous persisted tree byte-for-byte (10.15). */
  persistedTreeUnchanged: boolean;
  /** Error message shown on failure. */
  message?: string;
}

export const MOVE_STATE_PINS: readonly MoveStatePin[] = [
  { scenario: 'keyboard Move dialog relocates a folder within its library', matrixRow: 'move', trigger: 'keyboard-move-dialog', actor: { kind: 'folder', id: 'folder-vms' }, destination: { kind: 'folder', id: 'folder-lab' }, result: 'succeeds', locationAfter: 'under-destination', focusAfter: { kind: 'folder', id: 'folder-vms' }, persistedTreeUnchanged: false },
  { scenario: 'drag-and-drop relocates a connection through the same transaction', matrixRow: 'move', trigger: 'drag-drop', actor: { kind: 'connection', id: 'conn-bastion' }, destination: { kind: 'folder', id: 'folder-vms' }, result: 'succeeds', locationAfter: 'under-destination', focusAfter: { kind: 'connection', id: 'conn-bastion' }, persistedTreeUnchanged: false },
  { scenario: 'moving a folder into its own subtree fails validation and restores location and focus', matrixRow: 'move', trigger: 'keyboard-move-dialog', actor: { kind: 'folder', id: 'folder-work' }, destination: { kind: 'folder', id: 'folder-vms' }, result: 'fails', failureReason: 'destination-inside-actor-subtree', locationAfter: 'previous-location', focusAfter: { kind: 'folder', id: 'folder-work' }, persistedTreeUnchanged: true, message: 'Cannot move a folder into its own subtree.' },
  { scenario: 'a failed write rolls the drag back to the previous persisted location', matrixRow: 'move', trigger: 'drag-drop', actor: { kind: 'connection', id: 'conn-jump' }, destination: { kind: 'folder', id: 'folder-vms' }, result: 'fails', failureReason: 'persistence-failed', locationAfter: 'previous-location', focusAfter: { kind: 'connection', id: 'conn-jump' }, persistedTreeUnchanged: true, message: 'Move failed: the library file could not be written.' },
];

// ── Filter states (matrix row "Filter", Req 10.9) ─────────────────────

/**
 * Matching nodes plus ALL ancestors stay visible (in tree order);
 * expansion while filtering is temporary and never mutates the saved
 * expansion; clearing the filter restores the saved expansion exactly.
 */
export interface FilterStatePin {
  scenario: string;
  matrixRow: 'filter';
  query: string;
  matchedOn: 'name' | 'host' | 'tags';
  matched: NodeId[];
  requiredAncestors: NodeId[];
  /** Render order while the filter is active: matched ∪ ancestors. */
  visibleDuringFilter: NodeId[];
  /** Expansion applied to render the filtered set — temporary only. */
  temporaryExpansion: Record<string, boolean>;
  savedExpansionBefore: Record<string, boolean>;
  /** Must deep-equal `savedExpansionBefore` at every moment while filtering. */
  savedExpansionDuringFilter: Record<string, boolean>;
  /** Must deep-equal `savedExpansionBefore` after clearing the filter. */
  restoredExpansionAfterClear: Record<string, boolean>;
  /** Text shown when nothing matches. */
  emptyStateText?: string;
}

export const FILTER_STATE_PINS: readonly FilterStatePin[] = [
  {
    scenario: 'a nested name match reveals ancestors that were saved collapsed',
    matrixRow: 'filter',
    query: 'web',
    matchedOn: 'name',
    matched: [
      { kind: 'connection', id: 'conn-web-dev' },
      { kind: 'connection', id: 'conn-web-prod' },
    ],
    requiredAncestors: [
      { kind: 'library', id: 'lib-personal' },
      { kind: 'folder', id: 'folder-work' },
      { kind: 'folder', id: 'folder-vms' },
      { kind: 'library', id: 'lib-team' },
      { kind: 'folder', id: 'folder-prod' },
    ],
    visibleDuringFilter: [
      { kind: 'library', id: 'lib-personal' },
      { kind: 'folder', id: 'folder-work' },
      { kind: 'folder', id: 'folder-vms' },
      { kind: 'connection', id: 'conn-web-dev' },
      { kind: 'library', id: 'lib-team' },
      { kind: 'folder', id: 'folder-prod' },
      { kind: 'connection', id: 'conn-web-prod' },
    ],
    temporaryExpansion: { 'lib-personal': true, 'folder-work': true, 'folder-vms': true, 'lib-team': true, 'folder-prod': true },
    savedExpansionBefore: TREE_SAVED_EXPANSION_DEFAULT,
    savedExpansionDuringFilter: TREE_SAVED_EXPANSION_DEFAULT,
    restoredExpansionAfterClear: TREE_SAVED_EXPANSION_DEFAULT,
  },
  {
    scenario: 'a host match shows the connection with its ancestor path',
    matrixRow: 'filter',
    query: '10.0.0.14',
    matchedOn: 'host',
    matched: [{ kind: 'connection', id: 'conn-jump' }],
    requiredAncestors: [
      { kind: 'library', id: 'lib-personal' },
      { kind: 'folder', id: 'folder-lab' },
    ],
    visibleDuringFilter: [
      { kind: 'library', id: 'lib-personal' },
      { kind: 'folder', id: 'folder-lab' },
      { kind: 'connection', id: 'conn-jump' },
    ],
    temporaryExpansion: { 'lib-personal': true, 'folder-lab': true },
    savedExpansionBefore: TREE_SAVED_EXPANSION_DEFAULT,
    savedExpansionDuringFilter: TREE_SAVED_EXPANSION_DEFAULT,
    restoredExpansionAfterClear: TREE_SAVED_EXPANSION_DEFAULT,
  },
  {
    scenario: 'a tag-only match still shows the connection with its ancestors',
    matrixRow: 'filter',
    query: 'ssh',
    matchedOn: 'tags',
    matched: [{ kind: 'connection', id: 'conn-term' }],
    requiredAncestors: [
      { kind: 'library', id: 'lib-personal' },
      { kind: 'folder', id: 'folder-work' },
    ],
    visibleDuringFilter: [
      { kind: 'library', id: 'lib-personal' },
      { kind: 'folder', id: 'folder-work' },
      { kind: 'connection', id: 'conn-term' },
    ],
    temporaryExpansion: { 'lib-personal': true, 'folder-work': true },
    savedExpansionBefore: TREE_SAVED_EXPANSION_DEFAULT,
    savedExpansionDuringFilter: TREE_SAVED_EXPANSION_DEFAULT,
    restoredExpansionAfterClear: TREE_SAVED_EXPANSION_DEFAULT,
  },
  {
    scenario: 'no match clears the tree and shows an explicit empty state',
    matrixRow: 'filter',
    query: 'zzz-no-match',
    matchedOn: 'name',
    matched: [],
    requiredAncestors: [],
    visibleDuringFilter: [],
    temporaryExpansion: {},
    savedExpansionBefore: TREE_SAVED_EXPANSION_DEFAULT,
    savedExpansionDuringFilter: TREE_SAVED_EXPANSION_DEFAULT,
    restoredExpansionAfterClear: TREE_SAVED_EXPANSION_DEFAULT,
    emptyStateText: 'No matching connections',
  },
];

// ── Rail states (matrix row "Status/rail", Req 10.11/10.4/10.14) ──────

export interface RailEntryPin {
  library: NodeId;
  name: string;
  /** Textual aggregate status — in addition to any color/dot (10.4/10.14). */
  statusText: string;
  /** Full accessible name of the rail entry. */
  accessibleName: string;
}

/**
 * Explicit collapse to a narrow rail preserves all tree state
 * (selection + expansion) and the preference persists through
 * `AppSettings.sidebarCollapsed`.
 */
export interface RailStatePin {
  scenario: 'expanded-sidebar' | 'collapsed-rail';
  matrixRow: 'status-rail';
  sidebarCollapsed: boolean;
  /** Named toggle control (explicit collapse, 10.11). */
  toggleControlName: string;
  entries: RailEntryPin[];
  /** Tree state that must survive collapse/expand unchanged. */
  preserved: { selectedNodeId: NodeId | null; expanded: Record<string, boolean> };
}

const RAIL_PRESERVED_STATE = {
  selectedNodeId: { kind: 'connection', id: 'conn-bastion' } as NodeId,
  expanded: TREE_SAVED_EXPANSION_DEFAULT,
};

export const RAIL_STATE_PINS: readonly RailStatePin[] = [
  {
    scenario: 'expanded-sidebar',
    matrixRow: 'status-rail',
    sidebarCollapsed: false,
    toggleControlName: 'Collapse sidebar',
    entries: [
      { library: { kind: 'library', id: 'lib-personal' }, name: 'Personal Library', statusText: '1 of 4 connected', accessibleName: 'Personal Library, 1 of 4 connected' },
      { library: { kind: 'library', id: 'lib-team' }, name: 'Team Library', statusText: '1 of 4 connected', accessibleName: 'Team Library, 1 of 4 connected' },
    ],
    preserved: RAIL_PRESERVED_STATE,
  },
  {
    scenario: 'collapsed-rail',
    matrixRow: 'status-rail',
    sidebarCollapsed: true,
    toggleControlName: 'Expand sidebar',
    entries: [
      { library: { kind: 'library', id: 'lib-personal' }, name: 'Personal Library', statusText: '1 of 4 connected', accessibleName: 'Personal Library, 1 of 4 connected' },
      { library: { kind: 'library', id: 'lib-team' }, name: 'Team Library', statusText: '1 of 4 connected', accessibleName: 'Team Library, 1 of 4 connected' },
    ],
    preserved: RAIL_PRESERVED_STATE,
  },
];
