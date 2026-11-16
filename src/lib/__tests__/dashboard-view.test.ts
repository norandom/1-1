import { vi, describe, expect, it } from 'vitest';

// The dashboard listing comes from the existing 6.x IPC wrapper
// (`list_descendants` via src/lib/tauri.ts) — mocked here, node-env.
vi.mock('../tauri', () => ({ listDescendants: vi.fn() }));

import type { ConnectionSummary, LibraryTree, NodeId } from '../../types';
import { listDescendants } from '../tauri';
import {
  SESSION_STATUS_TEXT,
  TREE_AGGREGATE_STATUS_TEXT,
  UX_ACCEPTANCE_TREE,
  UX_SESSION_STATUSES,
} from '../ux-tree-states';
import { DASHBOARD_WITH_TABS_PINS, TAB_LIST_STATE_PINS } from '../ux-tab-states';
import {
  buildDashboardCards,
  buildDashboardModel,
  fetchDescendants,
  mainPanelFor,
  startViewEmptyState,
} from '../dashboard-view';

const lib = (id: string): NodeId => ({ kind: 'library', id });
const fol = (id: string): NodeId => ({ kind: 'folder', id });
const con = (id: string): NodeId => ({ kind: 'connection', id });

const summariesOf = (ids: string[]): ConnectionSummary[] =>
  ids.map((id) => UX_ACCEPTANCE_TREE.connections.find((c) => c.id === id)!);

const modelInput = (node: NodeId | null, summaries: ConnectionSummary[]) => ({
  tree: UX_ACCEPTANCE_TREE,
  selectedNodeId: node,
  statuses: UX_SESSION_STATUSES,
  summaries,
});

/** Folder + library with no connections at all (empty-state fixture). */
const EMPTY_DASHBOARD_TREE: LibraryTree = {
  libraries: [{ id: 'lib-empty', name: 'Empty Library', createdAt: '', updatedAt: '' }],
  folders: [{ id: 'folder-empty', libraryId: 'lib-empty', parentId: null, name: 'Empty folder' }],
  connections: [],
};

// ── fetchDescendants (Req 10.5 listing source: the 6.x IPC wrapper) ───

describe('fetchDescendants', () => {
  it('asks list_descendants for the node kind + id and returns its listing', async () => {
    const mock = vi.mocked(listDescendants).mockResolvedValue(summariesOf(['conn-bastion']));

    const out = await fetchDescendants(lib('lib-team'));

    expect(mock).toHaveBeenCalledWith('library', 'lib-team');
    expect(out).toEqual(summariesOf(['conn-bastion']));
  });

  it('propagates backend errors (unknown nodes) instead of an empty dashboard', async () => {
    vi.mocked(listDescendants).mockRejectedValue(new Error('unknown node'));

    await expect(fetchDescendants(fol('folder-nope'))).rejects.toThrow('unknown node');
    expect(vi.mocked(listDescendants)).toHaveBeenCalledWith('folder', 'folder-nope');
  });
});

// ── buildDashboardCards (Req 10.5 live status, 10.4 text never color) ─

describe('buildDashboardCards', () => {
  it('maps every descendant to a card with textual live status, preserving listing order', () => {
    const cards = buildDashboardCards(
      summariesOf(['conn-web-dev', 'conn-jump', 'conn-web-prod']),
      UX_SESSION_STATUSES
    );

    expect(cards.map((c) => c.connectionId)).toEqual([
      'conn-web-dev',
      'conn-jump',
      'conn-web-prod',
    ]);
    expect(cards[0]).toEqual({
      connectionId: 'conn-web-dev',
      name: 'web-dev-01',
      host: '10.0.0.11',
      status: 'connected',
      statusText: SESSION_STATUS_TEXT.connected,
    });
    expect(cards[1].statusText).toBe(SESSION_STATUS_TEXT.reconnecting);
    expect(cards[2].statusText).toBe(SESSION_STATUS_TEXT.error);
  });

  it("reads a connection without a session as Disconnected (design.md: that session's status or disconnected)", () => {
    const cards = buildDashboardCards(summariesOf(['conn-term']), UX_SESSION_STATUSES);
    expect(cards[0].status).toBe('disconnected');
    expect(cards[0].statusText).toBe('Disconnected');
  });
});

// ── buildDashboardModel (Req 10.5: nested included + aggregate) ───────

describe('buildDashboardModel', () => {
  it('a library lists every connection incl. nested folders, with the pinned aggregate', () => {
    const model = buildDashboardModel(
      modelInput(lib('lib-personal'), summariesOf(['conn-web-dev', 'conn-db-dev', 'conn-term', 'conn-jump']))
    )!;

    expect(model.title).toBe('Personal Library');
    expect(model.cards.map((c) => c.connectionId)).toEqual([
      'conn-web-dev',
      'conn-db-dev',
      'conn-term',
      'conn-jump',
    ]);
    expect(model.statusText).toBe(TREE_AGGREGATE_STATUS_TEXT['lib-personal']);
    expect(model.emptyState).toBeNull();
  });

  it('a folder lists its subtree incl. nested folders (10.5), pinned aggregate', () => {
    const work = buildDashboardModel(
      modelInput(fol('folder-work'), summariesOf(['conn-web-dev', 'conn-db-dev', 'conn-term']))
    )!;
    expect(work.title).toBe('Work');
    expect(work.statusText).toBe(TREE_AGGREGATE_STATUS_TEXT['folder-work']);

    const vms = buildDashboardModel(
      modelInput(fol('folder-vms'), summariesOf(['conn-web-dev', 'conn-db-dev']))
    )!;
    expect(vms.cards).toHaveLength(2);
    expect(vms.statusText).toBe(TREE_AGGREGATE_STATUS_TEXT['folder-vms']);
  });

  it('a selected connection node dashboards to exactly that connection (backend descendants semantics)', () => {
    const model = buildDashboardModel(
      modelInput(con('conn-bastion'), summariesOf(['conn-bastion']))
    )!;
    expect(model.cards.map((c) => c.connectionId)).toEqual(['conn-bastion']);
    expect(model.statusText).toBe('Connected');
  });

  it('a library/folder without connections is the no-connections empty state', () => {
    for (const node of [lib('lib-empty'), fol('folder-empty')]) {
      const model = buildDashboardModel({
        tree: EMPTY_DASHBOARD_TREE,
        selectedNodeId: node,
        statuses: {},
        summaries: [],
      })!;
      expect(model.cards).toEqual([]);
      expect(model.emptyState).toEqual({ kind: 'no-connections' });
    }
  });

  it('returns null when nothing is selected (the start view owns the panel)', () => {
    expect(buildDashboardModel(modelInput(null, []))).toBeNull();
  });
});

// ── mainPanelFor (Req 11.5: dashboard of the selection, or start) ─────

describe('mainPanelFor', () => {
  it('no selection → the start view', () => {
    expect(mainPanelFor(UX_ACCEPTANCE_TREE, null)).toEqual(TAB_LIST_STATE_PINS[1].mainPanel);
  });

  it('library selection → its dashboard (pinned dashboard-with-open-tab state)', () => {
    expect(mainPanelFor(UX_ACCEPTANCE_TREE, lib('lib-team'))).toEqual(
      TAB_LIST_STATE_PINS[2].mainPanel
    );
  });

  it('folder selection → its dashboard', () => {
    expect(mainPanelFor(UX_ACCEPTANCE_TREE, fol('folder-prod'))).toEqual({
      view: 'dashboard',
      node: { kind: 'folder', id: 'folder-prod' },
    });
  });

  it('a selection whose node vanished → the start view (stale selection is not a dashboard)', () => {
    expect(mainPanelFor(UX_ACCEPTANCE_TREE, fol('folder-deleted'))).toEqual({ view: 'start' });
    expect(mainPanelFor(EMPTY_DASHBOARD_TREE, lib('lib-personal'))).toEqual({ view: 'start' });
  });

  it('reproduces every pinned dashboard-with-tabs navigation (11.6 main-panel swaps)', () => {
    const pin = DASHBOARD_WITH_TABS_PINS[0];
    for (const step of pin.steps) {
      if (!('select' in step.userAction)) continue; // selectTab routing is task 8.6
      expect(mainPanelFor(UX_ACCEPTANCE_TREE, step.userAction.select)).toEqual(
        step.mainPanelAfter
      );
    }
  });
});

// ── startViewEmptyState (11.5 start view; no libraries at all) ────────

describe('startViewEmptyState', () => {
  it('a store with no libraries is the no-libraries empty state', () => {
    expect(startViewEmptyState({ libraries: [], folders: [], connections: [] })).toEqual({
      kind: 'no-libraries',
    });
    // Orphan folders/connections without a library are not a library list.
    expect(
      startViewEmptyState({ libraries: [], folders: EMPTY_DASHBOARD_TREE.folders, connections: [] })
    ).toEqual({ kind: 'no-libraries' });
  });

  it('null as soon as one library exists', () => {
    expect(startViewEmptyState(UX_ACCEPTANCE_TREE)).toBeNull();
  });
});
