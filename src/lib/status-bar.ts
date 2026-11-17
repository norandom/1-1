import type { ConnectionConfig, SessionInfo } from '../types';

import { SESSION_STATUS_TEXT } from './ux-tree-states';
import { isLiveSessionStatus, resolveSessionView, sessionRouteIdFromPath } from './session-route';

export function shouldAutoHideStatusBar(enabled: boolean, pathname: string, sessions: Iterable<SessionInfo>, connections: Pick<ConnectionConfig, 'id'>[]): boolean {
  if (!enabled) return false;
  const id = sessionRouteIdFromPath(pathname);
  return !!id && resolveSessionView(id, sessions, (connectionId) => connections.some((c) => c.id === connectionId)).kind !== 'stale';
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
