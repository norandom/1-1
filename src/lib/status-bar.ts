import type { SessionInfo } from '../types';

/**
 * Status-bar text derivation extracted from `StatusBar` (task 3.4
 * characterization): pure moves, rewired into the component with
 * identical semantics.
 *
 * `status` is deliberately part of the accepted shape: the current
 * implementation ignores it (the text claims "Connected to …" whenever a
 * session exists, whatever its lifecycle status) — pinned as a known
 * defect (B3-12) rather than frozen as correct behavior.
 */

export type StatusBarSession = Pick<
  SessionInfo,
  'connectionId' | 'width' | 'height' | 'status'
>;

export function buildStatusBarText(
  activeSession: StatusBarSession | null
): string {
  return activeSession
    ? `Connected to ${activeSession.connectionId} - ${activeSession.width}x${activeSession.height}`
    : 'No active connection';
}

export function sessionCountLabel(count: number): string {
  return `${count} active session${count !== 1 ? 's' : ''}`;
}
