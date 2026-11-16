import { describe, expect, it } from 'vitest';
import type { LibraryStoreStatus } from '../../types';
import type { ContextMenuAction } from '../tree-menu';
import {
  ALL_MUTATION_ACTIONS,
  MUTATING_MENU_ACTIONS,
  RECOVERY_BLOCKED_MESSAGE,
  blockedMenuActions,
  blockedMutationActions,
  isMenuActionBlocked,
  isMutationBlocked,
  isWriteBlocked,
  recoveryViewModel,
} from '../recovery';

/**
 * Task 8.7 (Req 10.12): the recovery gating model. While the store is
 * write-blocked EVERY mutation path is disabled/rejected, reads and
 * Connect stay available, and the instructions view-model presents the
 * backend recovery text with the preserved-originals note.
 */

const READY: LibraryStoreStatus = { status: 'ready' };
const BLOCKED: LibraryStoreStatus = {
  status: 'writeBlocked',
  reason: 'migration failed: unsupported schema version 99',
  instructions: 'Restore libraries.json (or the .backup copy) from disk, then choose Reload Library Store.',
};

describe('isWriteBlocked (task 8.7, Req 10.12)', () => {
  it('ready and unknown (null) statuses are not write-blocked', () => {
    expect(isWriteBlocked(READY)).toBe(false);
    expect(isWriteBlocked(null)).toBe(false);
  });

  it('the writeBlocked status is write-blocked', () => {
    expect(isWriteBlocked(BLOCKED)).toBe(true);
  });
});

describe('mutation gating decision (which actions are blocked per status)', () => {
  it('ready and unknown statuses block no mutation path', () => {
    expect(blockedMutationActions(READY)).toEqual([]);
    expect(blockedMutationActions(null)).toEqual([]);
    for (const action of ALL_MUTATION_ACTIONS) {
      expect(isMutationBlocked(READY, action)).toBe(false);
      expect(isMutationBlocked(null, action)).toBe(false);
    }
  });

  it('covers every store write surface in the taxonomy', () => {
    // The complete client-side mutation inventory: connection CRUD +
    // duplicate, folder CRUD, library rename/delete, move (8.4) and the
    // import wrappers. New surfaces must extend this list (and its gate).
    expect([...ALL_MUTATION_ACTIONS].sort()).toEqual(
      [
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
      ].sort()
    );
  });

  it('writeBlocked blocks EVERY mutation path (Req 10.12)', () => {
    expect([...blockedMutationActions(BLOCKED)].sort()).toEqual([...ALL_MUTATION_ACTIONS].sort());
    for (const action of ALL_MUTATION_ACTIONS) {
      expect(isMutationBlocked(BLOCKED, action)).toBe(true);
    }
  });
});

describe('context-menu gating (reads and Connect stay available)', () => {
  const EVERY_MENU_ACTION: ContextMenuAction[] = [
    'connect',
    'edit',
    'duplicate',
    'move',
    'delete',
    'new-connection',
    'new-folder',
    'rename',
  ];

  it('writeBlocked blocks every mutating menu action but never Connect', () => {
    expect([...blockedMenuActions(BLOCKED)].sort()).toEqual([...MUTATING_MENU_ACTIONS].sort());
    expect(blockedMenuActions(BLOCKED)).not.toContain('connect');
    for (const action of EVERY_MENU_ACTION) {
      expect(isMenuActionBlocked(BLOCKED, action)).toBe(action !== 'connect');
    }
  });

  it('ready blocks no menu action', () => {
    expect(blockedMenuActions(READY)).toEqual([]);
    for (const action of EVERY_MENU_ACTION) {
      expect(isMenuActionBlocked(READY, action)).toBe(false);
    }
  });

  it('exposes one pinned message for rejected mutation attempts', () => {
    expect(RECOVERY_BLOCKED_MESSAGE.length).toBeGreaterThan(0);
  });
});

describe('recovery instructions view-model (task 8.7, Req 10.12)', () => {
  it('is invisible while ready or the status is unknown', () => {
    for (const status of [READY, null] as const) {
      const model = recoveryViewModel(status);
      expect(model.visible).toBe(false);
      expect(model.reason).toBeNull();
      expect(model.instructions).toBeNull();
    }
  });

  it('presents the backend reason + instructions, the preserved-originals note and the reload action', () => {
    const model = recoveryViewModel(BLOCKED);
    expect(model.visible).toBe(true);
    expect(model.title?.length).toBeGreaterThan(0);
    expect(model.reason).toBe(BLOCKED.reason);
    expect(model.instructions).toBe(BLOCKED.instructions);
    // Originals are preserved on disk (Req 10.12) — the note says so.
    expect(model.preservedNote?.toLowerCase()).toContain('preserved');
    expect(model.reloadLabel?.length).toBeGreaterThan(0);
  });
});
