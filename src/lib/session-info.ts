import type { SessionInfo, SessionInfoWire } from '../types';

/** Normalize both event and invoke payloads before they enter UI state. */
export function normalizeSessionInfo(info: SessionInfoWire | SessionInfo): SessionInfo {
  const error = typeof info.status === 'object' ? info.status.error : null;
  return {
    ...info,
    status: typeof info.status === 'object' ? 'error' : info.status,
    connectedAt: info.connectedAt ?? undefined,
    lastError: info.lastError ?? error,
  };
}
