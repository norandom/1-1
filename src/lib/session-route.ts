/**
 * Task 8.6 (Req 11.4/11.8/11.9, design.md L396): session-id routing.
 * `/session/:sessionId` is THE session route and SessionView resolves
 * the connectionId THROUGH the session — never straight from the route
 * id (fixes drift D1: two sessions of one connection, or a route id
 * that is a session id, must never re-connect the wrong thing).
 */
import type { SessionInfo, SessionStatus } from '../types';

/** Session route — the single source of that string. */
export function sessionRoute(sessionId: string): string {
  return `/session/${encodeURIComponent(sessionId)}`;
}

/** The session id a pathname points at, or null for non-session routes. */
export function sessionRouteIdFromPath(pathname: string): string | null {
  const match = /^\/session\/([^/]+)\/?$/.exec(pathname);
  if (!match) return null;
  try {
    return decodeURIComponent(match[1]!);
  } catch {
    return match[1]!;
  }
}

/** Session statuses that cannot come back on their own (Req 11.9). */
export const TERMINAL_SESSION_STATUSES: readonly SessionStatus[] = [
  'disconnected',
  'error',
];

export function isTerminalSessionStatus(status: SessionStatus): boolean {
  return TERMINAL_SESSION_STATUSES.includes(status);
}

/** Session statuses an existing session is "open" in (Req 11.8). */
export function isLiveSessionStatus(status: SessionStatus): boolean {
  return !isTerminalSessionStatus(status);
}

/** How SessionView must treat the current `/session/:id` route. */
export type SessionViewResolution =
  | {
      /** A session exists for the route id: render it. */
      kind: 'view';
      sessionId: string;
      /** Resolved THROUGH the session (D1) — never from the route id. */
      connectionId: string;
      /** Terminal session: 7.1 restart keeps the id, same tab (11.9). */
      restart: boolean;
    }
  | {
      /** Transitional fresh connect: route id is a connection id with no
       * session yet; SessionView connects and replaces the URL with the
       * session id the backend returns. */
      kind: 'connect';
      connectionId: string;
    }
  | { kind: 'stale' };

/**
 * Resolve a `/session/:id` route against the live sessions. A route id
 * that is neither a session nor a known connection is stale (e.g. the
 * Back button after a tab close) and falls back to `/` (Req 11.5).
 */
export function resolveSessionView(
  routeId: string | undefined,
  sessions: Iterable<SessionInfo>,
  isKnownConnection: (id: string) => boolean
): SessionViewResolution {
  if (!routeId) return { kind: 'stale' };
  for (const session of sessions) {
    if (session.id === routeId) {
      return {
        kind: 'view',
        sessionId: session.id,
        connectionId: session.connectionId,
        restart: isTerminalSessionStatus(session.status),
      };
    }
  }
  if (isKnownConnection(routeId)) {
    return { kind: 'connect', connectionId: routeId };
  }
  return { kind: 'stale' };
}

/**
 * The existing session for a connection, any status (Req 11.8 live
 * focus; Req 11.9 terminal restart in the same tab), else undefined.
 */
export function sessionForConnection(
  sessions: Iterable<SessionInfo>,
  connectionId: string
): SessionInfo | undefined {
  for (const session of sessions) {
    if (session.connectionId === connectionId) return session;
  }
  return undefined;
}
