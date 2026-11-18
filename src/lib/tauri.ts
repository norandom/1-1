import { invoke } from '@tauri-apps/api/core';
import type { ViewportSize } from './viewport-size';
import type { Channel } from '@tauri-apps/api/core';
import type {
  CascadedIds,
  CheckedCredential,
  ConnectionConfig,
  ConnectionInput,
  ConnectionSummary,
  DeletionReport,
  Folder,
  Library,
  LibraryStoreStatus,
  LibraryTree,
  NodeKind,
  SessionInfo,
  SessionInfoWire,
  ServerIdentityReview,
  AppSettings,
  KeyEvent,
  MouseEvent as RdpMouseEvent,
} from '../types';
import { normalizeConnection } from './host-profile';
import { normalizeSessionInfo } from './session-info';
import { invalidateCredentialCheck, invalidateLibraryCredentialChecks } from '../stores/credentialCheckStore';
import type { FrameMessage, FrameReceipt } from './frame-protocol';
import { useSessionStore } from '../stores/sessionStore';

// ── Connection Management ───────────────────────────────────────────

// Webview invocations can overlap. Discard old evidence immediately, then retire
// any check that raced the save before Rust published its effective revision.
async function changingCheckedProfile<T>(invalidate: () => void, change: () => Promise<T>): Promise<T> {
  invalidate();
  try { return await change(); }
  finally { invalidate(); }
}

export async function listConnections(): Promise<ConnectionConfig[]> {
  try {
    return (await invoke<ConnectionConfig[]>('list_connections')).map(normalizeConnection);
  } catch (e) {
    console.error('Failed to list connections:', e);
    return [];
  }
}

export async function getConnection(id: string): Promise<ConnectionConfig | null> {
  try {
    const connection = await invoke<ConnectionConfig | null>('get_connection', { id });
    return connection ? normalizeConnection(connection) : null;
  } catch (e) {
    console.error('Failed to get connection:', e);
    return null;
  }
}

export async function createConnection(
  config: ConnectionInput
): Promise<ConnectionConfig> {
  return normalizeConnection(await invoke<ConnectionConfig>('create_connection', { config }));
}

export async function updateConnection(
  id: string,
  config: Partial<ConnectionConfig>
): Promise<ConnectionConfig> {
  const updated = await changingCheckedProfile(() => invalidateCredentialCheck(id),
    () => invoke<ConnectionConfig>('update_connection', { id, config }));
  return normalizeConnection(updated);
}

export async function deleteConnection(id: string): Promise<DeletionReport> {
  // Req 1.5/10.17/10.18 (task 7.3): coordinated deletion — the backend
  // stops the connection's session (bounded) before persisting and
  // cleans the credential best-effort only after; the report carries
  // the exact counts incl. stopped sessions and orphaned credentials.
  return changingCheckedProfile(() => invalidateCredentialCheck(id), () => invoke<DeletionReport>('delete_connection', { id }));
}

/// Req 10.19: duplicate a connection (non-secret settings, same folder,
/// fresh id + deterministic copy name; never a copied credential).
export async function duplicateConnection(id: string): Promise<ConnectionConfig> {
  return normalizeConnection(await invoke<ConnectionConfig>('duplicate_connection', { id }));
}

export function reviewServerIdentity(id: string): Promise<ServerIdentityReview> {
  return invoke('review_server_identity', { id });
}

export function testSavedCredentials(connectionId: string): Promise<CheckedCredential | null> {
  return invoke('test_saved_credentials', { connectionId });
}

export function confirmServerIdentity(selection: string): Promise<void> {
  return invoke('confirm_server_identity', { selection });
}

export function cancelServerIdentity(selection: string): Promise<void> {
  return invoke('cancel_server_identity', { selection });
}

export async function testConnection(host: string, port: number = 3389): Promise<boolean> {
  try {
    return await invoke<boolean>('test_connection', { host, port });
  } catch {
    return false;
  }
}

export async function measureLatency(host: string, port: number): Promise<number | null> {
  try {
    return await invoke<number | null>('measure_latency', { host, port });
  } catch {
    return null;
  }
}

// ── Session Management ──────────────────────────────────────────────

export async function connect(connectionId: string, frameChannel: Channel<FrameMessage>, initialSize?: ViewportSize): Promise<string> {
  return await invoke<string>('connect', { connectionId, frameChannel, ...(initialSize ? { initialSize } : {}) });
}

export async function frameConsumed(sessionId: string, receipt: FrameReceipt, refresh = false): Promise<void> {
  await invoke('frame_consumed', { sessionId, receipt, refresh });
}

export async function disconnect(sessionId: string): Promise<void> {
  await invoke('disconnect', { sessionId });
}

export async function sessionViewReturned(sessionId: string): Promise<void> {
  await invoke('session_view_returned', { sessionId });
}

export async function resumeDisplay(sessionId: string): Promise<void> {
  await invoke('resume_display', { sessionId });
}

export async function pauseDisplay(sessionId: string): Promise<void> {
  await invoke('pause_display', { sessionId });
}

export async function cancelReconnect(sessionId: string): Promise<void> {
  await invoke('cancel_reconnect', { sessionId });
}

export async function listSessions(): Promise<SessionInfo[]> {
  try {
    return (await invoke<SessionInfoWire[]>('list_sessions')).map(normalizeSessionInfo);
  } catch {
    return [];
  }
}

export async function getSessionInfo(sessionId: string): Promise<SessionInfo | null> {
  try {
    const info = await invoke<SessionInfoWire | null>('get_session_info', { sessionId });
    return info ? normalizeSessionInfo(info) : null;
  } catch {
    return null;
  }
}

// Starting a paused gesture may flush several physical modifiers at once.
// Wait for admission of each event to this actor before submitting the next.
const pendingInput = new Map<string, { connectedAt?: string; done: Promise<void> }>();
function orderedInput(sessionId: string, submit: () => Promise<void>): Promise<void> {
  const session = useSessionStore.getState().sessions.get(sessionId);
  const prior = pendingInput.get(sessionId);
  if (!session) return prior ? Promise.resolve() : submit();
  if (session.status !== 'connected') return Promise.resolve();
  const { connectedAt } = session;
  const preceding = prior && prior.connectedAt === connectedAt ? prior.done : undefined;
  if (!preceding && !session.displayPaused) return submit();
  const submitIfCurrent = () => {
    const current = useSessionStore.getState().sessions.get(sessionId);
    if (current?.status === 'connected' && current.connectedAt === connectedAt) return submit();
  };
  // A failed command must not swallow a later physical key release.
  const submitted = preceding ? preceding.then(submitIfCurrent, submitIfCurrent) : submit();
  pendingInput.set(sessionId, { connectedAt, done: submitted });
  return submitted.finally(() => {
    if (pendingInput.get(sessionId)?.done === submitted) pendingInput.delete(sessionId);
  });
}

export async function sendKeyEvent(sessionId: string, event: KeyEvent): Promise<void> {
  await orderedInput(sessionId, () => invoke('send_key_event', {
    sessionId,
    keyCode: event.keyCode,
    isDown: event.isDown,
  }));
}

export async function sendMouseEvent(sessionId: string, event: RdpMouseEvent): Promise<void> {
  const submit = () => invoke<void>('send_mouse_event', {
    sessionId,
    x: event.x,
    y: event.y,
    button: event.button ? String(event.button) : null,
    eventType: event.eventType,
    scrollDelta: event.scrollDelta ?? 0,
  });
  if (event.eventType === 'move') await submit();
  else await orderedInput(sessionId, submit);
}

export async function resizeSession(
  sessionId: string,
  width: number,
  height: number
): Promise<void> {
  await invoke('resize_session', { sessionId, width, height });
}

export async function getFrame(sessionId: string): Promise<string | null> {
  try {
    return await invoke<string>('get_frame', { sessionId });
  } catch {
    return null;
  }
}

export async function getConnectionPassword(id: string): Promise<string | null> {
  try {
    return await invoke<string>('get_connection_password', { id });
  } catch {
    return null;
  }
}

// ── Imports (task 6.6: library/folder destination, always non-mock) ──

export async function importRdpFile(
  path: string,
  libraryId?: string,
  folderId?: string | null
): Promise<ConnectionConfig> {
  return normalizeConnection(await invoke<ConnectionConfig>('import_rdp_file', {
    path,
    libraryId: libraryId ?? null,
    folderId: folderId ?? null,
  }));
}

export async function importSshConfig(
  libraryId?: string,
  folderId?: string | null
): Promise<number> {
  return await invoke<number>('import_ssh_config', {
    libraryId: libraryId ?? null,
    folderId: folderId ?? null,
  });
}

// ── Clipboard ────────────────────────────────────────────────────────
export async function clipboardPaste(sessionId: string, epoch: string): Promise<void> {
  await invoke('clipboard_paste', { sessionId, epoch });
}
export async function clipboardSync(sessionId: string, epoch: string, payload: import('../types').ClipboardPayload | null): Promise<void> {
  await invoke('clipboard_sync', { sessionId, epoch, payload });
}
export async function clipboardContext(sessionId: string): Promise<{epoch: string; direction: import('../types').HostProfile['clipboardDirection']}> {
  return invoke('clipboard_context', { sessionId });
}

export async function clipboardWrite(sessionId: string, text: string): Promise<void> {
  await invoke('clipboard_write', { sessionId, text });
}

export async function clipboardRead(sessionId: string): Promise<string> {
  return await invoke<string>('clipboard_read', { sessionId });
}

// ── Settings ────────────────────────────────────────────────────────

export async function getSettings(): Promise<AppSettings> {
  try {
    return await invoke<AppSettings>('get_settings');
  } catch {
    return {
      theme: 'system',
      defaultPort: 3389,
      autoReconnect: true,
      autoHideStatusBar: false,
      showPerformanceHud: false,
      sidebarCollapsed: false,
      defaultResolution: '1920x1080',
      colorDepth: 32,
      quality: 80,
      reconnectTimeout: 5000,
      connectionTimeout: 10000,
    };
  }
}

export async function updateSettings(settings: Partial<AppSettings>): Promise<AppSettings> {
  return await invoke<AppSettings>('update_settings', { settings });
}

// ── Library tree (tasks 6.4/6.5, Req 10.1/10.2/10.3/10.5/10.8/10.15) ─
// Thin typed wrappers over commands::library; CRUD/move errors reject
// (the UI must surface failures, Req 10.15) instead of swallowing them.

export async function createLibrary(name: string): Promise<Library> {
  return await invoke<Library>('create_library', { name });
}

export async function renameLibrary(id: string, name: string): Promise<Library> {
  return await invoke<Library>('rename_library', { id, name });
}

export async function createFolder(
  libraryId: string,
  parentId: string | null,
  name: string
): Promise<Folder> {
  return await invoke<Folder>('create_folder', { libraryId, parentId, name });
}

export async function renameFolder(id: string, name: string): Promise<Folder> {
  return await invoke<Folder>('rename_folder', { id, name });
}

// Req 10.16/10.18 (task 7.2): delete a folder subtree; every descendant
// connection moves to the same library's root (never deleted). Returns
// the exact cascaded counts/IDs for the confirmation UI; errors reject.
export async function deleteFolder(id: string): Promise<CascadedIds> {
  return await invoke<CascadedIds>('delete_folder', { id });
}

// Req 10.2/10.17/10.18 (task 7.3): delete a library with every folder and
// connection filed in it. The backend stops the affected sessions
// (bounded, serialized against connect) before persisting one cascaded
// transaction and cleans credentials only after it; the report carries
// the exact counts, the stopped sessions and any orphaned credentials.
export async function deleteLibrary(id: string): Promise<DeletionReport> {
  const report = await changingCheckedProfile(() => invalidateLibraryCredentialChecks(id),
    () => invoke<DeletionReport>('delete_library', { id }));
  for (const deleted of report.cascaded.deletedConnectionIds) invalidateCredentialCheck(deleted);
  return report;
}

export async function moveNode(
  kind: NodeKind,
  id: string,
  targetKind: NodeKind,
  targetId: string
): Promise<void> {
  return await invoke<void>('move_node', { kind, id, targetKind, targetId });
}

export async function listTree(): Promise<LibraryTree> {
  return await invoke<LibraryTree>('list_tree');
}

export async function listDescendants(kind: NodeKind, id: string): Promise<ConnectionSummary[]> {
  return await invoke<ConnectionSummary[]>('list_descendants', { kind, id });
}

export async function getLibraryStatus(): Promise<LibraryStoreStatus> {
  return await invoke<LibraryStoreStatus>('get_library_status');
}

export async function reloadLibraryStore(): Promise<LibraryStoreStatus> {
  return changingCheckedProfile(invalidateLibraryCredentialChecks, () => invoke<LibraryStoreStatus>('reload_library_store'));
}

export async function clipboardLocalAcquire(sessionId: string, epoch: string): Promise<string> { return invoke('clipboard_local_acquire', { sessionId, epoch }); }
export async function clipboardLocalRelease(lease: string): Promise<void> { return invoke('clipboard_local_release', { lease }); }
export async function clipboardLocalRead(lease: string): Promise<import('../types').ClipboardPayload> { return invoke('clipboard_local_read', { lease }); }
export async function clipboardLocalWrite(lease: string, payload: import('../types').ClipboardPayload): Promise<void> { return invoke('clipboard_local_write', { lease, payload }); }

export interface UpdateInfo { version: string; notes: string | null }
export interface UpdateCheck { currentVersion: string; update: UpdateInfo | null }
export interface UpdateProgress { downloaded: number; total: number | null; phase: 'downloading' | 'installing' }
export async function checkForUpdates(): Promise<UpdateCheck> {
  return invoke<UpdateCheck>('check_for_updates');
}
export async function installUpdate(onProgress: (progress: UpdateProgress) => void): Promise<void> {
  const { Channel: ProgressChannel } = await import('@tauri-apps/api/core');
  const progress = new ProgressChannel<UpdateProgress>();
  progress.onmessage = onProgress;
  return invoke<void>('install_update', { progress });
}
