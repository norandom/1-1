/**
 * Pinned UX acceptance states for document tabs and the dashboard /
 * start-view main panel — the executable form of ux-acceptance.md
 * (task 8.1).
 *
 * Covers the design.md "UX acceptance matrix" rows **Dashboard with
 * tabs** and **Close tab** (Req 11.1-11.3, 11.5-11.7). Pure data +
 * type contracts only: no component, store or routing behavior lives
 * here. Tasks 8.5-8.6 build `DashboardView`/`DocumentTabs`/`StartView`
 * against these states; their tests import this module as the shared
 * contract. Keep this file and ux-acceptance.md in exact agreement.
 */
import type { MainPanelView, NodeId, SessionStatus } from '../types';
import { SESSION_STATUS_TEXT, type DesignMatrixRow } from './ux-tree-states';

/** One pinned session tab (identity per session, Req 11.1/11.4/11.7). */
export interface PinnedTab {
  /** Session id — tab identity is stable per session (11.4). */
  tabId: string;
  connectionId: string;
  /** Named tab (11.7). */
  name: string;
  /** Textual live status alongside any color (10.4). */
  statusText: string;
  /** Named close control — closing disconnects the session (11.3/11.7). */
  closeControlName: string;
}

/**
 * A pinned tab-strip state: tab list + active underline (11.1), the
 * keyboard-focused tab (visible focus, 11.7), the pointer-hovered tab
 * (hover affordance, 11.1) and where the main panel points.
 */
export interface TabListStatePin {
  scenario: string;
  matrixRow: DesignMatrixRow;
  tabs: PinnedTab[];
  activeTabId: string | null;
  keyboardFocusTabId: string | null;
  pointerHoverTabId: string | null;
  mainPanel: MainPanelView;
}

const tab = (tabId: string, connectionId: string, name: string, status: SessionStatus): PinnedTab => ({
  tabId,
  connectionId,
  name,
  statusText: SESSION_STATUS_TEXT[status],
  closeControlName: `Close ${name}`,
});

export const TAB_LIST_STATE_PINS: readonly TabListStatePin[] = [
  {
    scenario: 'two open sessions: active tab underlined, hover affordance on the other',
    matrixRow: 'dashboard-with-tabs',
    tabs: [tab('session-bastion', 'conn-bastion', 'bastion', 'connected'), tab('session-jump', 'conn-jump', 'jump-host', 'reconnecting')],
    activeTabId: 'session-jump',
    keyboardFocusTabId: 'session-jump',
    pointerHoverTabId: 'session-bastion',
    mainPanel: { view: 'session', sessionId: 'session-jump' },
  },
  {
    scenario: 'no session open and nothing selected: the start view shows',
    matrixRow: 'dashboard-with-tabs',
    tabs: [],
    activeTabId: null,
    keyboardFocusTabId: null,
    pointerHoverTabId: null,
    mainPanel: { view: 'start' },
  },
  {
    scenario: 'a library is selected while a session stays open: dashboard shows, tab remains',
    matrixRow: 'dashboard-with-tabs',
    tabs: [tab('session-bastion', 'conn-bastion', 'bastion', 'connected')],
    activeTabId: null,
    keyboardFocusTabId: 'session-bastion',
    pointerHoverTabId: null,
    mainPanel: { view: 'dashboard', node: { kind: 'library', id: 'lib-team' } },
  },
];

/**
 * Req 11.6 pin: selecting libraries/folders while sessions are open
 * swaps the main panel to the dashboard WITHOUT disconnecting sessions
 * or removing tabs; selecting a tab returns to that exact session.
 */
export interface DashboardWithTabsStep {
  description: string;
  userAction: { select: NodeId } | { selectTab: string };
  mainPanelAfter: MainPanelView;
  tabsAfter: string[];
  sessionStatusesAfter: Record<string, SessionStatus>;
}

export interface DashboardWithTabsPin {
  scenario: string;
  matrixRow: 'dashboard-with-tabs';
  openSessions: Array<{ connectionId: string; status: SessionStatus }>;
  steps: readonly DashboardWithTabsStep[];
}

export const DASHBOARD_WITH_TABS_PINS: readonly DashboardWithTabsPin[] = [
  {
    scenario: 'two live sessions survive dashboard navigation and return exactly',
    matrixRow: 'dashboard-with-tabs',
    openSessions: [
      { connectionId: 'conn-bastion', status: 'connected' },
      { connectionId: 'conn-jump', status: 'reconnecting' },
    ],
    steps: [
      {
        description: 'select the Production folder in the tree',
        userAction: { select: { kind: 'folder', id: 'folder-prod' } },
        mainPanelAfter: { view: 'dashboard', node: { kind: 'folder', id: 'folder-prod' } },
        tabsAfter: ['session-bastion', 'session-jump'],
        sessionStatusesAfter: { 'session-bastion': 'connected', 'session-jump': 'reconnecting' },
      },
      {
        description: 'select the Personal Library in the tree',
        userAction: { select: { kind: 'library', id: 'lib-personal' } },
        mainPanelAfter: { view: 'dashboard', node: { kind: 'library', id: 'lib-personal' } },
        tabsAfter: ['session-bastion', 'session-jump'],
        sessionStatusesAfter: { 'session-bastion': 'connected', 'session-jump': 'reconnecting' },
      },
      {
        description: 'select the bastion tab: that exact session returns',
        userAction: { selectTab: 'session-bastion' },
        mainPanelAfter: { view: 'session', sessionId: 'session-bastion' },
        tabsAfter: ['session-bastion', 'session-jump'],
        sessionStatusesAfter: { 'session-bastion': 'connected', 'session-jump': 'reconnecting' },
      },
    ],
  },
];

/**
 * Req 11.7/11.3 pin: closing a tab disconnects that session, removes
 * exactly that tab, and focus lands deterministically on the adjacent
 * tab — the next one if it exists, else the previous one — or on the
 * dashboard when a tree node is selected, else on the start view.
 */
export type CloseTabFocusAfter =
  | { focus: 'adjacent-tab'; tabId: string; adjacency: 'next' | 'previous' }
  | { focus: 'dashboard'; node: NodeId }
  | { focus: 'start-view' };

export interface CloseTabPin {
  scenario: string;
  matrixRow: 'close-tab';
  tabsBefore: string[];
  closingTabId: string;
  tabsAfter: string[];
  /** Sessions disconnected by the close — exactly the closed one (11.3). */
  disconnects: string[];
  focusAfter: CloseTabFocusAfter;
}

export const CLOSE_TAB_PINS: readonly CloseTabPin[] = [
  {
    scenario: 'closing a middle tab focuses the next adjacent tab',
    matrixRow: 'close-tab',
    tabsBefore: ['session-bastion', 'session-jump', 'session-webprod'],
    closingTabId: 'session-jump',
    tabsAfter: ['session-bastion', 'session-webprod'],
    disconnects: ['session-jump'],
    focusAfter: { focus: 'adjacent-tab', tabId: 'session-webprod', adjacency: 'next' },
  },
  {
    scenario: 'closing the last tab focuses the previous adjacent tab',
    matrixRow: 'close-tab',
    tabsBefore: ['session-bastion', 'session-jump', 'session-webprod'],
    closingTabId: 'session-webprod',
    tabsAfter: ['session-bastion', 'session-jump'],
    disconnects: ['session-webprod'],
    focusAfter: { focus: 'adjacent-tab', tabId: 'session-jump', adjacency: 'previous' },
  },
  {
    scenario: 'closing the only tab with a tree node selected focuses its dashboard',
    matrixRow: 'close-tab',
    tabsBefore: ['session-bastion'],
    closingTabId: 'session-bastion',
    tabsAfter: [],
    disconnects: ['session-bastion'],
    focusAfter: { focus: 'dashboard', node: { kind: 'folder', id: 'folder-prod' } },
  },
  {
    scenario: 'closing the only tab with nothing selected focuses the start view',
    matrixRow: 'close-tab',
    tabsBefore: ['session-bastion'],
    closingTabId: 'session-bastion',
    tabsAfter: [],
    disconnects: ['session-bastion'],
    focusAfter: { focus: 'start-view' },
  },
];
