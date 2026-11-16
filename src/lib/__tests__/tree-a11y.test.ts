import { describe, expect, it } from 'vitest';

import type { SessionInfo, SessionStatus } from '../../types';
import {
  SESSION_STATUS_TEXT,
  TREE_AGGREGATE_STATUS_TEXT,
  TREE_A11Y_STATE,
  TREE_SAVED_EXPANSION_DEFAULT,
  UX_ACCEPTANCE_TREE,
  UX_SESSION_STATUSES,
} from '../ux-tree-states';
import {
  aggregateStatusText,
  buildTreeA11yNodes,
  connectionStatusText,
  descendantConnectionIds,
  statusesFromSessions,
  treeItemAria,
  treeItemLabel,
} from '../tree-a11y';

function makeSession(
  id: string,
  connectionId: string,
  status: SessionStatus
): SessionInfo {
  return {
    id,
    connectionId,
    connectionName: connectionId,
    host: '127.0.0.1',
    status,
    fps: 0,
    latency: 0,
    bandwidth: 0,
    width: 1920,
    height: 1080,
    reconnectAttempts: 0,
    maxReconnectAttempts: 0,
  };
}

const FIXTURE_INPUT = {
  tree: UX_ACCEPTANCE_TREE,
  statuses: UX_SESSION_STATUSES,
  expanded: TREE_SAVED_EXPANSION_DEFAULT,
  selectedNodeId: { kind: 'connection', id: 'conn-bastion' } as const,
};

// ── Per-connection textual status (Req 10.4/10.14) ───────────────────

describe('connectionStatusText', () => {
  it('uses the pinned text for every SessionStatus', () => {
    for (const [status, text] of Object.entries(SESSION_STATUS_TEXT)) {
      expect(connectionStatusText(status as SessionStatus)).toBe(text);
    }
  });

  it('shows the disconnected text when no session exists', () => {
    // conn-term has no session in the fixture overlay.
    expect(UX_SESSION_STATUSES['conn-term']).toBeUndefined();
    expect(connectionStatusText(undefined)).toBe('Disconnected');
  });
});

describe('statusesFromSessions', () => {
  it('keyes live statuses by connection id from the session store shape', () => {
    const sessions = [
      makeSession('sess-1', 'conn-bastion', 'connected'),
      makeSession('sess-2', 'conn-jump', 'reconnecting'),
    ];
    expect(statusesFromSessions(sessions)).toEqual({
      'conn-bastion': 'connected',
      'conn-jump': 'reconnecting',
    });
  });

  it('keeps the first session per connection (one session per connection, 11.9)', () => {
    const sessions = [
      makeSession('sess-1', 'conn-bastion', 'error'),
      makeSession('sess-2', 'conn-bastion', 'connected'),
    ];
    expect(statusesFromSessions(sessions)).toEqual({ 'conn-bastion': 'error' });
  });

  it('accepts a Map like the session store holds', () => {
    const map = new Map<string, SessionInfo>();
    map.set('sess-1', makeSession('sess-1', 'conn-web-prod', 'error'));
    expect(statusesFromSessions(map.values())).toEqual({ 'conn-web-prod': 'error' });
  });
});

// ── Aggregate status for containers (Req 10.4/10.14) ─────────────────

describe('descendantConnectionIds', () => {
  it('counts every descendant connection, nested included', () => {
    expect(descendantConnectionIds(UX_ACCEPTANCE_TREE, 'lib-personal')).toHaveLength(4);
    expect(descendantConnectionIds(UX_ACCEPTANCE_TREE, 'folder-work')).toHaveLength(3);
    expect(descendantConnectionIds(UX_ACCEPTANCE_TREE, 'folder-vms')).toHaveLength(2);
    expect(descendantConnectionIds(UX_ACCEPTANCE_TREE, 'folder-lab')).toHaveLength(1);
    expect(descendantConnectionIds(UX_ACCEPTANCE_TREE, 'lib-team')).toHaveLength(4);
    expect(descendantConnectionIds(UX_ACCEPTANCE_TREE, 'folder-prod')).toHaveLength(3);
  });
});

describe('aggregateStatusText', () => {
  it('reproduces every pinned "<n> of <m> connected" text', () => {
    for (const [containerId, expected] of Object.entries(TREE_AGGREGATE_STATUS_TEXT)) {
      expect(aggregateStatusText(UX_ACCEPTANCE_TREE, containerId, UX_SESSION_STATUSES)).toBe(
        expected
      );
    }
  });

  it('counts only connected sessions', () => {
    // lib-personal: web-dev connected, db-dev connecting, term none, jump reconnecting → 1 of 4.
    expect(aggregateStatusText(UX_ACCEPTANCE_TREE, 'lib-personal', UX_SESSION_STATUSES)).toBe(
      '1 of 4 connected'
    );
  });
});

// ── Pinned visible-tree a11y state (Req 10.3/10.13/10.14) ────────────

describe('buildTreeA11yNodes', () => {
  it('reproduces TREE_A11Y_STATE exactly (names, levels, expansion, selection, status)', () => {
    expect(buildTreeA11yNodes(FIXTURE_INPUT)).toEqual(TREE_A11Y_STATE);
  });

  it('hides the children of collapsed containers (folder-vms is saved collapsed)', () => {
    const ids = buildTreeA11yNodes(FIXTURE_INPUT).map((n) => n.node.id);
    expect(ids).not.toContain('conn-web-dev');
    expect(ids).not.toContain('conn-db-dev');
    expect(ids).toContain('folder-vms');
  });

  it('exposes expansion as null for connections', () => {
    const conn = buildTreeA11yNodes(FIXTURE_INPUT).find(
      (n) => n.node.kind === 'connection'
    );
    expect(conn?.expanded).toBeNull();
  });

  it('marks only the selected node', () => {
    const selected = buildTreeA11yNodes(FIXTURE_INPUT).filter((n) => n.selected);
    expect(selected).toHaveLength(1);
    expect(selected[0].node).toEqual({ kind: 'connection', id: 'conn-bastion' });
  });
});

describe('treeItemLabel', () => {
  it('composes the accessible name as "name, status" so status is never color-only', () => {
    expect(treeItemLabel(TREE_A11Y_STATE[0])).toBe('Personal Library, 1 of 4 connected');
    const jump = TREE_A11Y_STATE.find((n) => n.node.id === 'conn-jump')!;
    expect(treeItemLabel(jump)).toBe('jump-host, Reconnecting…');
  });
});

describe('treeItemAria', () => {
  it('exposes level, selection and expansion for containers', () => {
    expect(treeItemAria(TREE_A11Y_STATE[0])).toEqual({
      'aria-level': 1,
      'aria-selected': false,
      'aria-expanded': true,
    });
  });

  it('omits aria-expanded for connections', () => {
    const conn = TREE_A11Y_STATE.find((n) => n.node.kind === 'connection')!;
    const aria = treeItemAria(conn);
    expect(aria).toEqual({ 'aria-level': conn.level, 'aria-selected': conn.selected });
    expect('aria-expanded' in aria).toBe(false);
  });
});
