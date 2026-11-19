import type { FolderShareState, FolderShareStatus, ShareMode } from '../types';

/** Portable RDP preferred DOS name; mirrors `is_portable_share_name`. */
export function isPortableShareName(name: string): boolean {
  return /^[A-Za-z0-9_-]{1,7}$/.test(name);
}

export function shareModeLabel(mode: ShareMode): string {
  return mode === 'readWrite' ? 'Read and write' : 'Read-only';
}

export const SHARE_TIMING_NOTE = 'Changes apply on the next connection. To end access during a session, disconnect.';

/** Where to see what a session did with the folder. */
export const SHARE_SESSION_NOTE = 'The bottom bar shows whether each session offered the folder and whether the server used it.';

/** One line for the saved-host editor and the dialog's committed state. */
export function folderShareSummary(state: FolderShareState | null): string {
  if (!state) return 'Checking folder sharing…';
  if (!state.supported) return 'Folder sharing is available in the Windows build.';
  if (state.error) return state.error;
  if (!state.grant) return 'No folder is shared with this host.';
  const { folder, shareName, mode } = state.grant;
  return `${folder} as “${shareName}”, ${shareModeLabel(mode).toLowerCase()}.`;
}

/** The bottom bar's short word for a session's folder share. */
export function shareStatusLabel(status: FolderShareStatus): string {
  switch (status.state) {
    case 'off': return 'off';
    case 'unsupported': return 'not available';
    case 'unverified': case 'folderUnavailable': case 'unknown': return 'not shared';
    case 'negotiating': return 'connecting';
    case 'offered': return 'offered';
    case 'accepted': return 'accepted';
    case 'accessObserved': return 'in use';
    case 'refused': return 'refused';
    case 'ended': return 'ended';
  }
}

/** What the evidence says, for the share's menu. */
export function shareStatusDetail(status: FolderShareStatus): string {
  switch (status.state) {
    case 'off': return 'No folder is shared with this host.';
    case 'unsupported': case 'unverified': case 'folderUnavailable': case 'unknown': return status.reason;
    case 'negotiating': return 'The drive is set up for this connection and not yet announced to the server.';
    case 'offered': return 'The drive was announced. The server has not answered yet.';
    case 'accepted': return 'The server accepted the drive. No file on it has been used yet.';
    case 'accessObserved': return 'The server used files on the drive successfully.';
    case 'refused': return `The server refused the drive (${status.status}). That alone does not say why; a server policy may block drive redirection.`;
    case 'ended': return 'The connection that carried the drive ended.';
  }
}
