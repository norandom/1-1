/**
 * Task 8.6 (Req 11.1-11.3, 11.7): the DocumentTabs view-model — the
 * pure derivation the hoisted strip renders. Tab identity is the
 * session id (11.4); states are pinned by the 8.1 fixtures
 * (TAB_LIST_STATE_PINS / CLOSE_TAB_PINS in `ux-tab-states.ts`) and the
 * tests keep this derivation in exact agreement with them. Rendering
 * lives in `src/components/session/DocumentTabs.tsx` and stays thin.
 */
import type { NodeId, SessionInfo } from '../types';
import type { CloseTabFocusAfter } from './ux-tab-states';
import { SESSION_STATUS_TEXT } from './ux-tree-states';
import { sessionRoute, sessionRouteIdFromPath } from './session-route';

/** One session tab — shape mirrors the pinned `PinnedTab` contract. */
export interface SessionTabModel {
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
 * One tab per session, in session (open-order) sequence. The name
 * prefers the connection store, then the session's own backend name,
 * then the session id.
 */
export function buildSessionTabs(
  sessions: Iterable<SessionInfo>,
  connectionNames: Readonly<Record<string, string>>
): SessionTabModel[] {
  const tabs: SessionTabModel[] = [];
  for (const session of sessions) {
    const name =
      connectionNames[session.connectionId] ||
      session.connectionName ||
      session.id;
    tabs.push({
      tabId: session.id,
      connectionId: session.connectionId,
      name,
      statusText: SESSION_STATUS_TEXT[session.status],
      closeControlName: `Close ${name}`,
    });
  }
  return tabs;
}

/**
 * The active tab is the session the route points at (11.2) — dashboards
 * and the start view have no active tab (11.5/11.6), and a transitional
 * connection-id route activates nothing.
 */
export function activeTabIdFromPath(
  pathname: string,
  tabIds: readonly string[]
): string | null {
  const routeId = sessionRouteIdFromPath(pathname);
  if (!routeId) return null;
  return tabIds.includes(routeId) ? routeId : null;
}

/**
 * Req 11.3/11.7, CLOSE_TAB_PINS semantics: closing a tab disconnects
 * that session and focus lands on the adjacent tab — the next one if it
 * exists, else the previous one — or, with no tabs left, on the selected
 * node's dashboard, else the start view. Never on the closed tab.
 */
export function resolveCloseFocus(
  tabIds: readonly string[],
  closingTabId: string,
  selectedNode: NodeId | null
): CloseTabFocusAfter {
  const index = tabIds.indexOf(closingTabId);
  if (index < 0) {
    return selectedNode ? { focus: 'dashboard', node: selectedNode } : { focus: 'start-view' };
  }
  const next = tabIds[index + 1];
  if (next !== undefined) {
    return { focus: 'adjacent-tab', tabId: next, adjacency: 'next' };
  }
  const previous = tabIds[index - 1];
  if (previous !== undefined) {
    return { focus: 'adjacent-tab', tabId: previous, adjacency: 'previous' };
  }
  return selectedNode ? { focus: 'dashboard', node: selectedNode } : { focus: 'start-view' };
}

/** Where each close-focus outcome navigates: the tab's session route, or `/`. */
export function closeFocusRoute(focus: CloseTabFocusAfter): string {
  return focus.focus === 'adjacent-tab' ? sessionRoute(focus.tabId) : '/';
}

/** Keyboard moves over the strip (Req 11.7): arrows wrap, Home/End jump. */
export type TabFocusKey = 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End';

export function nextTabFocus(
  tabIds: readonly string[],
  currentTabId: string | null,
  key: TabFocusKey
): string | null {
  if (tabIds.length === 0) return null;
  const first = tabIds[0]!;
  const last = tabIds[tabIds.length - 1]!;
  const index = currentTabId === null ? -1 : tabIds.indexOf(currentTabId);
  if (key === 'Home') return first;
  if (key === 'End') return last;
  if (index < 0) {
    // No (or stale) focus: arrows enter the strip from either end.
    return key === 'ArrowRight' ? first : last;
  }
  if (key === 'ArrowRight') {
    return tabIds[(index + 1) % tabIds.length]!;
  }
  return tabIds[(index - 1 + tabIds.length) % tabIds.length]!;
}

/** Accessible name for a tab: its name plus the textual status (11.7, 10.4). */
export function tabAccessibleName(tab: SessionTabModel): string {
  return `${tab.name}, ${tab.statusText}`;
}
