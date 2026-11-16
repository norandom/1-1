import { vi, describe, expect, it, beforeEach } from 'vitest';

// In-memory localStorage, installed before the store module is imported:
// zustand persist reads the storage once at store creation.
const localStorageStub = vi.hoisted(() => {
  const backing = new Map<string, string>();
  const storage: Storage = {
    getItem: (key: string) => (backing.has(key) ? backing.get(key)! : null),
    setItem: (key: string, value: string) => {
      backing.set(key, String(value));
    },
    removeItem: (key: string) => {
      backing.delete(key);
    },
    clear: () => {
      backing.clear();
    },
    key: (index: number) => Array.from(backing.keys())[index] ?? null,
    get length() {
      return backing.size;
    },
  };
  Object.defineProperty(globalThis, 'localStorage', {
    value: storage,
    configurable: true,
    writable: true,
  });
  return storage;
});

// libraryStore imports the tauri wrappers; none run at import time.
vi.mock('../../lib/tauri', () => ({
  listTree: vi.fn(),
  listConnections: vi.fn().mockResolvedValue([]),
  moveNode: vi.fn().mockResolvedValue(undefined),
}));

import type { NodeId, SessionInfo, SessionStatus } from '../../types';
import { DASHBOARD_WITH_TABS_PINS } from '../../lib/ux-tab-states';
import { UX_ACCEPTANCE_TREE } from '../../lib/ux-tree-states';
import { mainPanelFor } from '../../lib/dashboard-view';
import { useLibraryStore } from '../libraryStore';
import { useSessionStore } from '../sessionStore';

// Task 8.5 (Req 11.6): selecting libraries/folders while sessions are
// open swaps the main panel to the dashboard WITHOUT disconnecting
// sessions or removing tabs — pinned here at store level against
// DASHBOARD_WITH_TABS_PINS (the tab strip itself is task 8.6).

const pin = DASHBOARD_WITH_TABS_PINS[0];

function liveSession(
  sessionId: string,
  connectionId: string,
  status: SessionStatus
): SessionInfo {
  return {
    id: sessionId,
    connectionId,
    connectionName: sessionId,
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

beforeEach(() => {
  localStorageStub.clear();
  // Exact pinned tab ids (session-bastion / session-jump) with the pinned
  // statuses (connected / reconnecting), keyed by session id per Req 11.4.
  const sessions = new Map<string, SessionInfo>(
    pin.openSessions.map((open) => {
      const sessionId = `session-${open.connectionId.replace('conn-', '')}`;
      return [sessionId, liveSession(sessionId, open.connectionId, open.status)] as const;
    })
  );
  useSessionStore.setState({ sessions, activeSessionId: 'session-jump' });
  useLibraryStore.setState({
    tree: UX_ACCEPTANCE_TREE,
    selectedNodeId: null,
    filter: '',
  });
});

describe('selectNode never disturbs live sessions (Req 11.6)', () => {
  it('every pinned dashboard navigation keeps both sessions exactly as they were', () => {
    const before = useSessionStore.getState().sessions;
    const beforeActive = useSessionStore.getState().activeSessionId;

    for (const step of pin.steps) {
      if (!('select' in step.userAction)) continue; // selectTab routing is 8.6
      const node: NodeId = step.userAction.select;

      useLibraryStore.getState().selectNode(node);

      // The dashboard swap is real: selection drives the main panel.
      expect(useLibraryStore.getState().selectedNodeId).toEqual(node);
      expect(mainPanelFor(useLibraryStore.getState().tree, node)).toEqual(step.mainPanelAfter);

      // Nothing disconnected, nothing removed: selectNode must not touch
      // sessionStore at all — not even recreate the Map or its entries.
      const after = useSessionStore.getState().sessions;
      expect(after).toBe(before);
      expect(Array.from(after.keys()).sort()).toEqual([...step.tabsAfter].sort());
      for (const [sessionId, expectedStatus] of Object.entries(step.sessionStatusesAfter)) {
        expect(after.get(sessionId)?.status, `${step.description}: ${sessionId}`).toBe(
          expectedStatus
        );
        expect(after.get(sessionId)).toBe(before.get(sessionId));
      }
      expect(useSessionStore.getState().activeSessionId).toBe(beforeActive);
    }
  });

  it('selection survives unrelated session updates (status changes flow, tabs stay)', () => {
    useLibraryStore.getState().selectNode({ kind: 'folder', id: 'folder-prod' });

    // A live status update arrives for one session (reconnecting → connected).
    useSessionStore.getState().updateSession('session-jump', { status: 'connected' });

    // The dashboard selection is untouched by session traffic.
    expect(useLibraryStore.getState().selectedNodeId).toEqual({
      kind: 'folder',
      id: 'folder-prod',
    });
    // Both sessions still exist — one updated, one identical.
    const sessions = useSessionStore.getState().sessions;
    expect(sessions.size).toBe(2);
    expect(sessions.get('session-jump')?.status).toBe('connected');
    expect(sessions.get('session-bastion')?.status).toBe('connected');
  });

  it('deselecting (null) returns to the start view without touching sessions', () => {
    useLibraryStore.getState().selectNode({ kind: 'library', id: 'lib-personal' });
    const before = useSessionStore.getState().sessions;

    useLibraryStore.getState().selectNode(null);

    expect(mainPanelFor(useLibraryStore.getState().tree, null)).toEqual({ view: 'start' });
    expect(useSessionStore.getState().sessions).toBe(before);
  });
});
