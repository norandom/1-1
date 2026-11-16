import { describe, expect, it } from 'vitest';

import type { SessionInfo, SessionStatus } from '../../types';
import {
  TERMINAL_SESSION_STATUSES,
  isLiveSessionStatus,
  isTerminalSessionStatus,
  resolveSessionView,
  sessionForConnection,
  sessionRoute,
  sessionRouteIdFromPath,
} from '../session-route';

// Task 8.6 (Req 11.4/11.8/11.9, design.md L396): the session route is
// `/session/:sessionId` and SessionView resolves the connectionId
// THROUGH the session — never straight from the route id (D1 fix).

function session(
  id: string,
  connectionId: string,
  status: SessionStatus
): SessionInfo {
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

const knownConnections = new Set(['conn-bastion', 'conn-jump']);
const isKnownConnection = (id: string) => knownConnections.has(id);

describe('sessionRoute', () => {
  it('routes by session id — the single source of that string', () => {
    expect(sessionRoute('session-bastion')).toBe('/session/session-bastion');
  });
});

describe('sessionRouteIdFromPath', () => {
  it('extracts the session id from a session route', () => {
    expect(sessionRouteIdFromPath('/session/session-bastion')).toBe('session-bastion');
    expect(sessionRouteIdFromPath('/session/abc%2F123')).toBe('abc/123');
  });

  it('non-session routes yield null', () => {
    expect(sessionRouteIdFromPath('/')).toBeNull();
    expect(sessionRouteIdFromPath('/settings')).toBeNull();
    expect(sessionRouteIdFromPath('/session')).toBeNull();
    expect(sessionRouteIdFromPath('/session/')).toBeNull();
    expect(sessionRouteIdFromPath('/session/a/details')).toBeNull();
  });
});

describe('terminal/live status partition', () => {
  it('terminal means disconnected or error; live means connecting/connected/reconnecting (11.8/11.9)', () => {
    expect(TERMINAL_SESSION_STATUSES).toEqual(['disconnected', 'error']);
    expect(TERMINAL_SESSION_STATUSES.every(isTerminalSessionStatus)).toBe(true);
    expect(isTerminalSessionStatus('connected')).toBe(false);
    const liveStatuses: SessionStatus[] = ['connecting', 'connected', 'reconnecting'];
    expect(liveStatuses.every(isLiveSessionStatus)).toBe(true);
    expect(isLiveSessionStatus('error')).toBe(false);
    expect(isLiveSessionStatus('disconnected')).toBe(false);
  });
});

describe('resolveSessionView', () => {
  it('resolves connectionId THROUGH the session, never from the route id (D1, 11.4)', () => {
    const sessions = [
      session('session-bastion', 'conn-bastion', 'connected'),
      session('session-jump', 'conn-jump', 'reconnecting'),
    ];
    expect(resolveSessionView('session-bastion', sessions, isKnownConnection)).toEqual({
      kind: 'view',
      sessionId: 'session-bastion',
      connectionId: 'conn-bastion',
      restart: false,
    });
    expect(resolveSessionView('session-jump', sessions, isKnownConnection)).toEqual({
      kind: 'view',
      sessionId: 'session-jump',
      connectionId: 'conn-jump',
      restart: false,
    });
  });

  it('every live status is a plain view; every terminal status is a restart in the same tab (11.8/11.9)', () => {
    for (const status of ['connecting', 'connected', 'reconnecting'] as const) {
      const r = resolveSessionView('s1', [session('s1', 'c1', status)], isKnownConnection);
      expect(r).toMatchObject({ kind: 'view', restart: false });
    }
    for (const status of TERMINAL_SESSION_STATUSES) {
      const r = resolveSessionView('s1', [session('s1', 'c1', status)], isKnownConnection);
      expect(r).toMatchObject({ kind: 'view', sessionId: 's1', restart: true });
    }
  });

  it('a route id that is a known connection but has no session yet is a fresh connect', () => {
    expect(resolveSessionView('conn-bastion', [], isKnownConnection)).toEqual({
      kind: 'connect',
      connectionId: 'conn-bastion',
    });
  });

  it('unknown or missing route ids are stale', () => {
    expect(resolveSessionView('session-gone', [], isKnownConnection)).toEqual({ kind: 'stale' });
    expect(resolveSessionView(undefined, [], isKnownConnection)).toEqual({ kind: 'stale' });
  });

  it('a session id wins when ids could be ambiguous', () => {
    const sessions = [session('conn-bastion', 'conn-bastion', 'connected')];
    expect(resolveSessionView('conn-bastion', sessions, isKnownConnection)).toMatchObject({
      kind: 'view',
      sessionId: 'conn-bastion',
      connectionId: 'conn-bastion',
    });
  });
});

describe('sessionForConnection (Req 11.8/11.9)', () => {
  it('finds the existing session for a connection regardless of status, else undefined', () => {
    const sessions = [
      session('session-bastion', 'conn-bastion', 'connected'),
      session('session-jump', 'conn-jump', 'error'),
    ];
    expect(sessionForConnection(sessions, 'conn-bastion')?.id).toBe('session-bastion');
    expect(sessionForConnection(sessions, 'conn-jump')?.id).toBe('session-jump');
    expect(sessionForConnection(sessions, 'conn-other')).toBeUndefined();
    expect(sessionForConnection([], 'conn-bastion')).toBeUndefined();
  });
});
