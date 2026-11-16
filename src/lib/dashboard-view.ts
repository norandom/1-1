import type {
  ConnectionSummary,
  LibraryTree,
  MainPanelView,
  NodeId,
  SessionStatus,
} from '../types';
import { listDescendants } from './tauri';
import { SESSION_STATUS_TEXT } from './ux-tree-states';

/**
 * Task 8.5 (Req 10.5/10.6/11.5/11.6): the pure dashboard / start-view
 * view-model. The listing source is the existing 6.x IPC wrapper
 * (`list_descendants`, Req 10.5) — `fetchDescendants` is the only
 * side-effecting member; everything else transforms given data and is
 * node-tested in `src/lib/__tests__/dashboard-view.test.ts`.
 *
 * The render layer (`src/components/dashboard/`) stays thin: it feeds
 * store state + the fetched listing into these functions and renders the
 * result. Live statuses always come from `sessionStore` (design.md);
 * a connection without a session reads Disconnected.
 */

/** One dashboard card: a descendant connection with its live status. */
export interface DashboardCardModel {
  connectionId: string;
  name: string;
  host: string;
  status: SessionStatus;
  /** Textual status — never color-only (Req 10.4). */
  statusText: string;
}

/** Empty states pinned by the UX acceptance work (task 8.1 vocabulary). */
export type DashboardEmptyState = { kind: 'no-connections' } | null;
export type StartViewEmptyState = { kind: 'no-libraries' } | null;

export interface DashboardModel {
  node: NodeId;
  title: string;
  /** Aggregate for containers, single status for a connection node. */
  statusText: string;
  cards: DashboardCardModel[];
  emptyState: DashboardEmptyState;
}

/** Req 10.5 listing source: the backend `list_descendants` wrapper. */
export async function fetchDescendants(node: NodeId): Promise<ConnectionSummary[]> {
  return await listDescendants(node.kind, node.id);
}

/** Card list in listing order; absent session status = disconnected. */
export function buildDashboardCards(
  summaries: ConnectionSummary[],
  statuses: Record<string, SessionStatus>
): DashboardCardModel[] {
  return summaries.map((summary) => {
    const status = statuses[summary.id] ?? 'disconnected';
    return {
      connectionId: summary.id,
      name: summary.name,
      host: summary.host,
      status,
      statusText: SESSION_STATUS_TEXT[status],
    };
  });
}

function nodeExists(tree: LibraryTree, node: NodeId): boolean {
  if (node.kind === 'library') return tree.libraries.some((l) => l.id === node.id);
  if (node.kind === 'folder') return tree.folders.some((f) => f.id === node.id);
  return tree.connections.some((c) => c.id === node.id);
}

function nodeName(tree: LibraryTree, node: NodeId): string {
  if (node.kind === 'library') return tree.libraries.find((l) => l.id === node.id)?.name ?? node.id;
  if (node.kind === 'folder') return tree.folders.find((f) => f.id === node.id)?.name ?? node.id;
  return tree.connections.find((c) => c.id === node.id)?.name ?? node.id;
}

/**
 * `<connected> of <total> connected` over the dashboard's own cards —
 * the same TREE_AGGREGATE_STATUS_TEXT contract the tree shows.
 */
function aggregateText(cards: DashboardCardModel[]): string {
  const connected = cards.filter((c) => c.status === 'connected').length;
  return `${connected} of ${cards.length} connected`;
}

export interface DashboardModelInput {
  tree: LibraryTree;
  selectedNodeId: NodeId | null;
  /** Live statuses keyed by connection id (absent = no session). */
  statuses: Record<string, SessionStatus>;
  /** The fetched `descendants(node)` listing. */
  summaries: ConnectionSummary[];
}

/**
 * The dashboard for the selection, or null when nothing (valid) is
 * selected — then the start view owns the main panel (Req 11.5).
 */
export function buildDashboardModel(input: DashboardModelInput): DashboardModel | null {
  const { tree, selectedNodeId: node } = input;
  if (!node || !nodeExists(tree, node)) return null;

  const cards = buildDashboardCards(input.summaries, input.statuses);
  return {
    node,
    title: nodeName(tree, node),
    statusText:
      node.kind === 'connection' ? (cards[0]?.statusText ?? SESSION_STATUS_TEXT.disconnected) : aggregateText(cards),
    cards,
    emptyState: cards.length === 0 ? { kind: 'no-connections' } : null,
  };
}

/**
 * Req 11.5: where the main panel points — the dashboard of the selected
 * tree node, or the start view when nothing (valid) is selected. Session
 * views come from the router (`/session/:id`), not from selection.
 */
export function mainPanelFor(tree: LibraryTree, selectedNodeId: NodeId | null): MainPanelView {
  if (selectedNodeId && nodeExists(tree, selectedNodeId)) {
    return { view: 'dashboard', node: selectedNodeId };
  }
  return { view: 'start' };
}

/** The start view's empty state: no libraries at all. */
export function startViewEmptyState(tree: LibraryTree): StartViewEmptyState {
  return tree.libraries.length === 0 ? { kind: 'no-libraries' } : null;
}
