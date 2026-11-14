// Task 6.6 (Req 10.1/10.19): `libraryId`/`folderId`/`mock` mirror the v2
// Rust `ConnectionConfig` serde shape. They are optional here because this
// interface doubles as the create/update payload base (the Rust
// `ConnectionInput` serde-defaults them), so v1-era callers stay valid;
// every READ from the backend always includes all three.
export interface ConnectionConfig {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  domain: string;
  password?: string;
  colorAccent: string | null;
  tags: string[];
  displayWidth: number | null;
  displayHeight: number | null;
  createdAt: string;
  updatedAt: string;
  lastConnectedAt?: string;
  libraryId?: string;
  folderId?: string | null;
  mock?: boolean;
}

export type SessionStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'error';

export interface SessionInfo {
  id: string;
  connectionId: string;
  connectionName: string;
  host: string;
  status: SessionStatus;
  fps: number;
  latency: number;
  bandwidth: number;
  width: number;
  height: number;
  connectedAt?: string;
  reconnectAttempts: number;
  maxReconnectAttempts: number;
  lastError?: string | null;
}

export interface AppSettings {
  theme: 'system' | 'light' | 'dark';
  defaultPort: number;
  autoReconnect: boolean;
  showPerformanceHud: boolean;
  sidebarCollapsed: boolean;
  defaultResolution: string;
  colorDepth: number;
  quality: number;
  reconnectTimeout: number;
  connectionTimeout: number;
}

export interface KeyEvent {
  keyCode: number;
  isDown: boolean;
}

// ── Library tree (task 6.4, Req 10.1/10.2/10.3/10.5) ────────────────
// Mirrors src-tauri/src/store/library.rs (camelCase serde).

export interface Library {
  id: string;
  name: string;
  createdAt: string;
  updatedAt: string;
}

export interface Folder {
  id: string;
  libraryId: string;
  parentId: string | null;
  name: string;
}

export interface ConnectionSummary {
  id: string;
  name: string;
  host: string;
  libraryId: string;
  folderId: string | null;
}

export type NodeKind = 'library' | 'folder' | 'connection';

export interface LibraryTree {
  libraries: Library[];
  folders: Folder[];
  connections: ConnectionSummary[];
}

// Exact removal/move outcome of a destructive tree operation (task 7.2,
// Req 10.16/10.18): each list's length is the count the confirmation UI
// must state; folder deletion never deletes connections, so
// deletedConnectionIds is empty there and movedConnectionIds carries the
// re-filed descendants (IDs unchanged).
export interface CascadedIds {
  deletedFolderIds: string[];
  deletedConnectionIds: string[];
  movedConnectionIds: string[];
}

export type LibraryStoreStatus =
  | { status: 'ready' }
  | { status: 'writeBlocked'; reason: string; instructions: string };

export interface MouseEvent {
  x: number;
  y: number;
  button: number;
  eventType: 'move' | 'down' | 'up' | 'scroll';
  scrollDelta?: number;
}
