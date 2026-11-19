import { platform } from '@tauri-apps/plugin-os';
import type { ConnectionConfig, Library } from '../types';
import * as tauri from './tauri';
import { hostCommand, type HostImport } from './host-command';
import { useConnectionStore } from '../stores/connectionStore';
import { useLibraryStore } from '../stores/libraryStore';

function isWindows(): boolean {
  try {
    return platform() === 'windows';
  } catch {
    return navigator.userAgent.includes('Windows');
  }
}

/** Copy a saved host as a command line, with its library and confirmed key. */
export async function copyHostCommand(connectionId: string): Promise<string> {
  const host = useConnectionStore.getState().connections.find((c) => c.id === connectionId);
  if (!host) throw new Error('The host is no longer saved.');
  const library = useLibraryStore.getState().tree.libraries.find((l) => l.id === host.libraryId)?.name ?? null;
  // Without a confirmed key the command leaves it out; it can be added on import.
  const fingerprint = await tauri.confirmedFingerprint(connectionId).catch(() => null);
  const command = hostCommand(host, { library, fingerprint, windows: isWindows() });
  await tauri.copyText(command);
  return command;
}

/** The library an import files into: the named one, created when missing, or the first. */
async function targetLibrary(name: string | null): Promise<Library> {
  const libraries = useLibraryStore.getState().tree.libraries;
  if (name) {
    const existing = libraries.find((l) => l.name.toLowerCase() === name.toLowerCase());
    if (existing) return existing;
    return useLibraryStore.getState().createLibrary(name);
  }
  if (libraries[0]) return libraries[0];
  return useLibraryStore.getState().createLibrary('Hosts');
}

export type ImportOutcome =
  | { kind: 'saved'; host: ConnectionConfig; verified: boolean }
  | { kind: 'mismatch'; host: ConnectionConfig; observed: string };

/**
 * Save the imported host. With a fingerprint, read the server's key without
 * credentials (as Review server identity does) and confirm it only when it
 * matches; a mismatch leaves the key unconfirmed and the caller must not
 * connect.
 */
export async function importHost(value: HostImport, fingerprint: string | null): Promise<ImportOutcome> {
  const library = await targetLibrary(value.library);
  const host = await useConnectionStore.getState().createConnection({ ...value.input, libraryId: library.id, folderId: null });
  if (!fingerprint) return { kind: 'saved', host, verified: false };
  const review = await tauri.reviewServerIdentity(host.id);
  if (review.observedFingerprint !== fingerprint) {
    await tauri.cancelServerIdentity(review.selection).catch(() => {});
    return { kind: 'mismatch', host, observed: review.observedFingerprint };
  }
  await tauri.confirmServerIdentity(review.selection);
  return { kind: 'saved', host, verified: true };
}
