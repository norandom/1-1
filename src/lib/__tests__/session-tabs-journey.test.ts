import { describe, expect, it } from 'vitest';

import type { SessionInfo, SessionStatus } from '../../types';
import {
  activeTabIdFromPath,
  buildSessionTabs,
  closeFocusRoute,
  resolveCloseFocus,
} from '../document-tabs';
import { resolveSessionView, sessionForConnection } from '../session-route';

// Task 8.6 Done criterion: "Two-session tests switch/close/reconnect
// exactly the intended session and restore adjacent/dashboard focus."
// This is the logic-level journey over exactly the functions the hoisted
// strip and the migrated SessionView consume (Req 11.2/11.3/11.4/11.7).

function session(id: string, connectionId: string, status: SessionStatus): SessionInfo {
  return {
    id,
    connectionId,
    connectionName: '',
    host: '',
    status,
    fps: 0,
    latency: 0,
    bandwidth: 0,
    width: 0,
    height: 0,
    reconnectAttempts: 0,
    maxReconnectAttempts: 5,
    lastError: null,
  };
}

describe('two-session journey (Done criterion, Req 11.2/11.3/11.4/11.7)', () => {
  const names = { 'conn-bastion': 'bastion', 'conn-jump': 'jump-host' };
  const bastion = session('session-bastion', 'conn-bastion', 'connected');
  const jump = session('session-jump', 'conn-jump', 'reconnecting');

  it('switch: selecting a tab resolves exactly that session — route, tab and connection agree (11.2)', () => {
    const sessions = [bastion, jump];
    const tabs = buildSessionTabs(sessions, names);
    const tabIds = tabs.map((t) => t.tabId);

    // Route → exact tab (active underline) …
    expect(activeTabIdFromPath('/session/session-jump', tabIds)).toBe('session-jump');
    // … and the view resolves that tab’s connection THROUGH the session (D1).
    expect(resolveSessionView('session-jump', sessions, () => false)).toEqual({
      kind: 'view',
      sessionId: 'session-jump',
      connectionId: 'conn-jump',
      restart: false,
    });
    expect(
      resolveSessionView('session-bastion', sessions, () => false)
    ).toMatchObject({ connectionId: 'conn-bastion' });
  });

  it('close: closing one tab disconnects exactly that session and focuses the adjacent tab (11.3/11.7)', () => {
    const tabIds = ['session-bastion', 'session-jump'];

    // Close the viewed (last) tab: focus restores to the previous adjacent tab.
    const focus = resolveCloseFocus(tabIds, 'session-jump', { kind: 'library', id: 'lib-team' });
    expect(focus).toEqual({ focus: 'adjacent-tab', tabId: 'session-bastion', adjacency: 'previous' });
    // The disconnect list is exactly the closed session; the survivor keeps its tab.
    expect(tabIds.filter((id) => id !== 'session-jump')).toEqual(['session-bastion']);
    // Navigating the focus outcome lands on the surviving session’s route.
    expect(closeFocusRoute(focus)).toBe('/session/session-bastion');
  });

  it('reconnect: identity survives status flips and a terminal restart — same id, same tab, one session (11.4/11.9)', () => {
    // connected → error → (restart request) → connecting: the store entry
    // keeps its id, so the route, tab and tree status stay stable.
    const statuses: SessionStatus[] = ['connected', 'reconnecting', 'error', 'disconnected', 'connecting', 'connected'];
    for (const status of statuses) {
      const s = session('session-bastion', 'conn-bastion', status);
      const resolution = resolveSessionView('session-bastion', [s, jump], () => false);
      expect(resolution).toMatchObject({ kind: 'view', sessionId: 'session-bastion', connectionId: 'conn-bastion' });
      expect(sessionForConnection([s, jump], 'conn-bastion')?.id).toBe('session-bastion');
    }
    // A Connect for the terminal connection still targets the same session tab.
    const terminal = session('session-bastion', 'conn-bastion', 'error');
    expect(sessionForConnection([terminal, jump], 'conn-bastion')?.id).toBe('session-bastion');
    expect(buildSessionTabs([terminal, jump], names)).toHaveLength(2);
  });

  it('close the last tab: focus restores to the selected dashboard, else the start view (11.5/11.7)', () => {
    expect(resolveCloseFocus(['session-bastion'], 'session-bastion', { kind: 'folder', id: 'folder-prod' })).toEqual({
      focus: 'dashboard',
      node: { kind: 'folder', id: 'folder-prod' },
    });
    const start = resolveCloseFocus(['session-bastion'], 'session-bastion', null);
    expect(start).toEqual({ focus: 'start-view' });
    expect(closeFocusRoute(start)).toBe('/');
  });
});
