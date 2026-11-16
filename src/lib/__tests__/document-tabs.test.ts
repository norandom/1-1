import { describe, expect, it } from 'vitest';

import type { NodeId, SessionInfo, SessionStatus } from '../../types';
import {
  CLOSE_TAB_PINS,
  TAB_LIST_STATE_PINS,
} from '../ux-tab-states';
import { SESSION_STATUS_TEXT } from '../ux-tree-states';
import { resolveUrlAction } from '../session-attach';
import { resolveSessionView, sessionForConnection } from '../session-route';
import {
  activeTabIdFromPath,
  buildSessionTabs,
  closeFocusRoute,
  nextTabFocus,
  resolveCloseFocus,
  tabAccessibleName,
} from '../document-tabs';
// Task 8.6 (Req 11.1-11.3, 11.7): the DocumentTabs view-model. The pure
// derivation below is what the hoisted strip renders; its states are
// pinned by the 8.1 fixtures (TAB_LIST_STATE_PINS / CLOSE_TAB_PINS) and
// the tests here keep the derivation in exact agreement with them.

function session(
  id: string,
  connectionId: string,
  status: SessionStatus,
  connectionName = ''
): SessionInfo {
  return {
    id,
    connectionId,
    connectionName,
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

describe('buildSessionTabs (Req 11.1/11.7)', () => {
  it('derives one named tab per session, in session order, with textual status and a named close control', () => {
    const names: Record<string, string> = {
      'conn-bastion': 'bastion',
      'conn-jump': 'jump-host',
    };
    const tabs = buildSessionTabs(
      [
        session('session-bastion', 'conn-bastion', 'connected'),
        session('session-jump', 'conn-jump', 'reconnecting'),
      ],
      names
    );

    expect(tabs).toEqual([
      {
        tabId: 'session-bastion',
        connectionId: 'conn-bastion',
        name: 'bastion',
        statusText: 'Connected',
        closeControlName: 'Close bastion',
      },
      {
        tabId: 'session-jump',
        connectionId: 'conn-jump',
        name: 'jump-host',
        statusText: 'Reconnecting…',
        closeControlName: 'Close jump-host',
      },
    ]);
  });

  it('falls back to the session’s connectionName, then the session id, when the connection store has no name', () => {
    const tabs = buildSessionTabs(
      [
        session('session-named', 'conn-x', 'connected', 'backend-name'),
        session('session-bare', 'conn-y', 'connecting'),
      ],
      {}
    );
    expect(tabs[0]?.name).toBe('backend-name');
    expect(tabs[0]?.closeControlName).toBe('Close backend-name');
    expect(tabs[1]?.name).toBe('session-bare');
    expect(tabs[1]?.closeControlName).toBe('Close session-bare');
  });

  it('reproduces every TAB_LIST_STATE_PINS tab exactly (fixture contract)', () => {
    // pins carry textual status; reverse SESSION_STATUS_TEXT to seed sessions
    const statusForText = new Map<string, SessionStatus>(
      (Object.keys(SESSION_STATUS_TEXT) as SessionStatus[]).map((s) => [SESSION_STATUS_TEXT[s], s])
    );
    for (const pin of TAB_LIST_STATE_PINS) {
      const names: Record<string, string> = {};
      for (const t of pin.tabs) names[t.connectionId] = t.name;
      const sessions = pin.tabs.map((t) =>
        session(t.tabId, t.connectionId, statusForText.get(t.statusText) ?? 'connected')
      );
      const tabs = buildSessionTabs(sessions, names);
      expect(tabs, pin.scenario).toEqual(pin.tabs);
    }
  });

  it('exposes an accessible name carrying tab name and textual status (11.7)', () => {
    const tabs = buildSessionTabs([session('s1', 'c1', 'connected', 'web')], { c1: 'web' });
    const name = tabAccessibleName(tabs[0]!);
    expect(name).toContain('web');
    expect(name).toContain('Connected');
  });
});

describe('activeTabIdFromPath (Req 11.1/11.2/11.6)', () => {
  const tabIds = ['session-bastion', 'session-jump'];

  it('the active tab is the session the route points at', () => {
    expect(activeTabIdFromPath('/session/session-jump', tabIds)).toBe('session-jump');
    expect(activeTabIdFromPath('/session/session-bastion', tabIds)).toBe('session-bastion');
  });

  it('dashboard, start view, settings and unknown session routes have no active tab (11.5/11.6)', () => {
    expect(activeTabIdFromPath('/', tabIds)).toBeNull();
    expect(activeTabIdFromPath('/settings', tabIds)).toBeNull();
    expect(activeTabIdFromPath('/session/session-unknown', tabIds)).toBeNull();
  });

  // Task 8.6 remediation round 1: this row previously pinned the BROKEN
  // end state — a fresh connect's URL stayed on the transitional
  // connection-id route forever (the swap effect was dead), so the
  // session being viewed never got its active-tab underline. It now
  // pins the CORRECT behavior: the transitional route activates nothing
  // ONLY for the moment before the swap, and the swap replace-navigates
  // onto the session route, where the active tab resolves to exactly
  // the session being viewed (11.1/11.4).
  it('a transitional connect route is swapped for the session route, which then resolves the active tab (11.1/11.4)', () => {
    const existing = session('session-bastion', 'conn-bastion', 'connected');
    const resolution = resolveSessionView('conn-bastion', [], () => true);
    expect(resolution).toEqual({ kind: 'connect', connectionId: 'conn-bastion' });
    // Transitional tick only: no tab is active on the connection-id URL…
    expect(activeTabIdFromPath('/session/conn-bastion', tabIds)).toBeNull();
    // …then the URL swap lands on the session route…
    const urlAction = resolveUrlAction(
      resolution,
      sessionForConnection([existing], 'conn-bastion'),
      'session-bastion'
    );
    expect(urlAction).toEqual({ action: 'replace', route: '/session/session-bastion' });
    // …where the viewed session's tab is the active tab.
    expect(urlAction.action === 'replace' && activeTabIdFromPath(urlAction.route, tabIds)).toBe(
      'session-bastion'
    );
  });
});

describe('resolveCloseFocus (Req 11.3/11.7, CLOSE_TAB_PINS)', () => {
  const folder: NodeId = { kind: 'folder', id: 'folder-prod' };

  it('reproduces every CLOSE_TAB_PINS focus outcome exactly (fixture contract)', () => {
    for (const pin of CLOSE_TAB_PINS) {
      const selected = pin.focusAfter.focus === 'dashboard' ? pin.focusAfter.node : null;
      const focus = resolveCloseFocus(pin.tabsBefore, pin.closingTabId, selected);
      expect(focus, pin.scenario).toEqual(pin.focusAfter);
    }
  });

  it('focus never lands on the closed tab, and prefers next over previous', () => {
    const focus = resolveCloseFocus(['a', 'b', 'c'], 'b', null);
    expect(focus).toEqual({ focus: 'adjacent-tab', tabId: 'c', adjacency: 'next' });
    const last = resolveCloseFocus(['a', 'b', 'c'], 'c', null);
    expect(last).toEqual({ focus: 'adjacent-tab', tabId: 'b', adjacency: 'previous' });
  });

  it('routes each focus outcome: adjacent tab keeps the session route, dashboard/start go home', () => {
    expect(closeFocusRoute({ focus: 'adjacent-tab', tabId: 'session-jump', adjacency: 'next' })).toBe(
      '/session/session-jump'
    );
    expect(closeFocusRoute({ focus: 'adjacent-tab', tabId: 'session-jump', adjacency: 'previous' })).toBe(
      '/session/session-jump'
    );
    expect(closeFocusRoute({ focus: 'dashboard', node: folder })).toBe('/');
    expect(closeFocusRoute({ focus: 'start-view' })).toBe('/');
  });
});

describe('nextTabFocus (Req 11.7 keyboard navigation)', () => {
  const ids = ['session-bastion', 'session-jump', 'session-webprod'];

  it('ArrowRight/ArrowLeft step to neighbours and wrap at the ends', () => {
    expect(nextTabFocus(ids, 'session-bastion', 'ArrowRight')).toBe('session-jump');
    expect(nextTabFocus(ids, 'session-jump', 'ArrowLeft')).toBe('session-bastion');
    expect(nextTabFocus(ids, 'session-webprod', 'ArrowRight')).toBe('session-bastion');
    expect(nextTabFocus(ids, 'session-bastion', 'ArrowLeft')).toBe('session-webprod');
  });

  it('Home/End jump to the first/last tab', () => {
    expect(nextTabFocus(ids, 'session-jump', 'Home')).toBe('session-bastion');
    expect(nextTabFocus(ids, 'session-jump', 'End')).toBe('session-webprod');
  });

  it('from no (or stale) focus, arrows enter the strip deterministically', () => {
    expect(nextTabFocus(ids, null, 'ArrowRight')).toBe('session-bastion');
    expect(nextTabFocus(ids, null, 'ArrowLeft')).toBe('session-webprod');
    expect(nextTabFocus(ids, 'session-gone', 'ArrowRight')).toBe('session-bastion');
  });

  it('an empty strip has no focus target', () => {
    expect(nextTabFocus([], 'session-bastion', 'ArrowRight')).toBeNull();
    expect(nextTabFocus([], null, 'Home')).toBeNull();
  });
});
