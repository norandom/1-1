import type { SessionInfo } from '../types';

import { SESSION_STATUS_TEXT } from './ux-tree-states';
import { isLiveSessionStatus } from './session-route';

export type StatusBarSession = Pick<
  SessionInfo,
  'connectionId' | 'width' | 'height' | 'status'
>;

export function buildStatusBarText(
  activeSession: StatusBarSession | null
): string {
  if (!activeSession) return 'No active connection';
  const { status, connectionId, width, height } = activeSession;
  switch (status) {
    case 'connected': return `Connected to ${connectionId} - ${width}x${height}`;
    case 'connecting': return `Connecting to ${connectionId}`;
    case 'reconnecting': return `Reconnecting to ${connectionId}`;
    case 'error': return `Connection error - ${connectionId}`;
    case 'disconnected': return `Disconnected - ${connectionId}`;
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
