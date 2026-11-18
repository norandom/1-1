import type { ConnectionConfig, SessionInfo } from '../types';

import { SESSION_STATUS_TEXT } from './ux-tree-states';
import { isLiveSessionStatus, resolveSessionView, sessionRouteIdFromPath } from './session-route';
import { normalizeHostProfile } from './host-profile';


/**
 * Whether this route shows the bottom bar as an overlay that appears with the
 * top toolbar: a live or pending session whose saved host chose `withToolbar`.
 * Every other view keeps the ordinary status row.
 */
export function bottomBarFollowsToolbar(
  pathname: string,
  sessions: Iterable<SessionInfo>,
  connections: Pick<ConnectionConfig, 'id' | 'hostProfile'>[],
): boolean {
  const id = sessionRouteIdFromPath(pathname);
  if (!id) return false;
  const live = [...sessions];
  const view = resolveSessionView(id, live, (connectionId) => connections.some((c) => c.id === connectionId));
  if (view.kind === 'stale') return false;
  const connectionId = live.find((session) => session.id === id)?.connectionId ?? id;
  const profile = connections.find((c) => c.id === connectionId)?.hostProfile;
  return normalizeHostProfile(profile).bottomBar === 'withToolbar';
}

export type StatusBarSession = Pick<
  SessionInfo,
  'connectionId' | 'width' | 'height' | 'status'
>;

export function buildStatusBarText(
  activeSession: StatusBarSession | null,
  connection?: Pick<ConnectionConfig, 'name' | 'host'>,
): string {
  if (!activeSession) return 'No active connection';
  const { status, width, height } = activeSession;
  const hostLabel = connection?.name.trim() || connection?.host.trim() || 'Unknown host';
  switch (status) {
    case 'connected': return `Connected to ${hostLabel} - ${width}x${height}`;
    case 'connecting': return `Connecting to ${hostLabel}`;
    case 'reconnecting': return `Reconnecting to ${hostLabel}`;
    case 'error': return `Connection error - ${hostLabel}`;
    case 'disconnected': return `Disconnected - ${hostLabel}`;
  }
}

export function sessionCountLabel(count: number): string {
  return `${count} active session${count !== 1 ? 's' : ''}`;
}

export function statusBarIndicator(session: StatusBarSession | null) {
  const status = session?.status ?? 'disconnected';
  const color = status === 'connected' ? 'var(--color-success)'
    : status === 'error' ? 'var(--color-danger)'
    : status === 'disconnected' ? 'var(--color-text-muted)' : 'var(--color-warning)';
  return { label: SESSION_STATUS_TEXT[status], color, pulse: status === 'connected' };
}

export function liveSessionCount(sessions: Iterable<SessionInfo>): number {
  return [...sessions].filter((session) => isLiveSessionStatus(session.status)).length;
}
