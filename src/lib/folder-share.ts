import type { FolderShareState, ShareMode } from '../types';

/** Portable RDP preferred DOS name; mirrors `is_portable_share_name`. */
export function isPortableShareName(name: string): boolean {
  return /^[A-Za-z0-9_-]{1,7}$/.test(name);
}

export function shareModeLabel(mode: ShareMode): string {
  return mode === 'readWrite' ? 'Read and write' : 'Read-only';
}

export const SHARE_TIMING_NOTE = 'Changes apply on the next connection. To end access during a session, disconnect.';

/**
 * Folder sharing is saved now; offering the folder to a session arrives
 * with the drive attachment task. Say so rather than imply live access.
 */
export const SHARE_PREVIEW_NOTE = 'Preview: this build saves the folder choice. Sessions start offering the folder in a later build.';

/** One line for the saved-host editor and the dialog's committed state. */
export function folderShareSummary(state: FolderShareState | null): string {
  if (!state) return 'Checking folder sharing…';
  if (!state.supported) return 'Folder sharing is available in the Windows build.';
  if (state.error) return state.error;
  if (!state.grant) return 'No folder is shared with this host.';
  const { folder, shareName, mode } = state.grant;
  return `${folder} as “${shareName}”, ${shareModeLabel(mode).toLowerCase()}.`;
}
