import type { LibraryStoreStatus } from '../types';
import type { ContextMenuAction } from './tree-menu';

/**
 * Task 8.7 (Req 10.12): the recovery gating model. When the backend
 * store is write-blocked (failed load/migration) the original file and
 * existing backup are preserved on disk, EVERY mutation path is
 * disabled/rejected in the UI, and the persistent instructions view
 * (RecoveryBanner) shows the backend recovery text until a successful
 * `reload_library_store` restores valid data. Reads — tree listing from
 * the last-known state, dashboards, and Connect (sessions are not store
 * mutations) — stay available throughout.
 */

/** The writeBlocked variant payload (backend `StoreStatus`, camelCase serde). */
export type WriteBlockedStatus = Extract<LibraryStoreStatus, { status: 'writeBlocked' }>;

/** True iff the store is in write-blocked recovery (Req 10.12). */
export function isWriteBlocked(status: LibraryStoreStatus | null): status is WriteBlockedStatus {
  return status !== null && status.status === 'writeBlocked';
}

/**
 * The complete client-side mutation inventory. Every UI surface that
 * writes the library store maps to exactly one of these; a future
 * surface (e.g. the import wrappers, which have no UI yet) must extend
 * this list AND wire the gate.
 */
export type MutationAction =
  | 'create-connection' // ConnectionForm create save / New Connection CTAs
  | 'update-connection' // ConnectionForm edit save
  | 'duplicate-connection' // tree context menu Duplicate (10.19)
  | 'delete-connection' // tree context menu Delete (10.17)
  | 'create-folder' // tree context menu New Folder (10.2)
  | 'rename-node' // tree context menu Rename (library/folder, 10.7)
  | 'delete-folder' // tree context menu Delete (10.16)
  | 'delete-library' // tree context menu Delete (10.18)
  | 'move-node' // 8.4 Move dialog + drag (10.8/10.14)
  | 'import-connections'; // .rdp / ssh-config import wrappers (no surface yet)

export const ALL_MUTATION_ACTIONS: readonly MutationAction[] = [
  'create-connection',
  'update-connection',
  'duplicate-connection',
  'delete-connection',
  'create-folder',
  'rename-node',
  'delete-folder',
  'delete-library',
  'move-node',
  'import-connections',
];

/** Context-menu actions that mutate the store — everything but connect. */
export const MUTATING_MENU_ACTIONS: readonly ContextMenuAction[] = [
  'edit',
  'duplicate',
  'move',
  'delete',
  'new-connection',
  'new-folder',
  'rename',
];

/** The gating decision: which mutation paths are blocked for this status. */
export function blockedMutationActions(status: LibraryStoreStatus | null): readonly MutationAction[] {
  return isWriteBlocked(status) ? ALL_MUTATION_ACTIONS : [];
}

/** True when the given mutation path must be disabled/rejected. */
export function isMutationBlocked(status: LibraryStoreStatus | null, action: MutationAction): boolean {
  return blockedMutationActions(status).includes(action);
}

/**
 * Which context-menu actions are blocked. `connect` is never blocked:
 * connecting starts/loads a session, it does not mutate the store.
 */
export function blockedMenuActions(status: LibraryStoreStatus | null): readonly ContextMenuAction[] {
  return isWriteBlocked(status) ? MUTATING_MENU_ACTIONS : [];
}

/** True when the given context-menu action must be disabled. */
export function isMenuActionBlocked(status: LibraryStoreStatus | null, action: ContextMenuAction): boolean {
  return blockedMenuActions(status).includes(action);
}

/**
 * The one pinned message for every rejected mutation attempt while
 * blocked (store rejections and disabled-affordance toasts share it).
 */
export const RECOVERY_BLOCKED_MESSAGE =
  'Changes are disabled while the library store is in recovery. Restore the original library file (or its backup), then reload.';

/**
 * The preserved-originals note (Req 10.12): recovery never destroys
 * client-side data — the file and backup stay on disk untouched.
 */
export const RECOVERY_PRESERVED_NOTE =
  'Your original library file and the existing backup are preserved on disk. Nothing is changed or deleted while recovery is active.';

/** View-model for the persistent RecoveryBanner (title/reload label pins). */
export interface RecoveryViewModel {
  /** Derived from status alone — the banner has no dismiss control. */
  visible: boolean;
  title: string | null;
  /** Backend display text of the blocking error. */
  reason: string | null;
  /** Backend recovery instructions (shown until valid data is reloaded). */
  instructions: string | null;
  /** Preserved-originals note (RECOVERY_PRESERVED_NOTE while visible). */
  preservedNote: string | null;
  /** Reload action label (the only path out of recovery). */
  reloadLabel: string | null;
}

export const RECOVERY_VIEW_TITLE = 'Library store needs recovery';
export const RECOVERY_RELOAD_LABEL = 'Reload Library Store';

/**
 * Status → instructions view-model. Invisible unless write-blocked;
 * visible derives ONLY from the status, so the banner cannot be
 * dismissed while the store is blocked (Req 10.12).
 */
export function recoveryViewModel(status: LibraryStoreStatus | null): RecoveryViewModel {
  if (!isWriteBlocked(status)) {
    return {
      visible: false,
      title: null,
      reason: null,
      instructions: null,
      preservedNote: null,
      reloadLabel: null,
    };
  }
  return {
    visible: true,
    title: RECOVERY_VIEW_TITLE,
    reason: status.reason,
    instructions: status.instructions,
    preservedNote: RECOVERY_PRESERVED_NOTE,
    reloadLabel: RECOVERY_RELOAD_LABEL,
  };
}
