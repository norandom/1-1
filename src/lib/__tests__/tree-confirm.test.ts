import { describe, expect, it } from 'vitest';

import type { DeletionReport, SessionInfo, SessionStatus } from '../../types';
import { UX_ACCEPTANCE_TREE } from '../ux-tree-states';
import {
  buildDeletionConfirmation,
  buildDeletionOutcomeToast,
  buildDuplicationNotice,
  deletionCountsFromReport,
  predictConnectionDeletion,
  predictFolderDeletion,
  predictLibraryDeletion,
} from '../tree-confirm';

function makeSession(id: string, connectionId: string, status: SessionStatus): SessionInfo {
  return {
    id,
    connectionId,
    connectionName: connectionId,
    host: '127.0.0.1',
    status,
    fps: 0,
    latency: 0,
    bandwidth: 0,
    width: 1920,
    height: 1080,
    reconnectAttempts: 0,
    maxReconnectAttempts: 0,
  };
}

const SESSIONS: SessionInfo[] = [
  makeSession('sess-bastion', 'conn-bastion', 'connected'),
  makeSession('sess-webdev', 'conn-web-dev', 'connected'),
  makeSession('sess-jump', 'conn-jump', 'reconnecting'),
];

// ── Count prediction from real tree + session state (Req 10.18) ──────

describe('predictConnectionDeletion', () => {
  it('deletes exactly that connection and stops exactly its sessions', () => {
    expect(predictConnectionDeletion('conn-bastion', SESSIONS)).toEqual({
      cascaded: {
        deletedFolderIds: [],
        deletedConnectionIds: ['conn-bastion'],
        movedConnectionIds: [],
      },
      stoppedSessionIds: ['sess-bastion'],
    });
  });

  it('reports zero stopped sessions when the connection has none', () => {
    expect(predictConnectionDeletion('conn-quick', SESSIONS).stoppedSessionIds).toEqual([]);
  });
});

describe('predictFolderDeletion (Req 10.16: move, never delete)', () => {
  it('removes the subtree folders and moves every descendant connection to the library root', () => {
    expect(predictFolderDeletion('folder-work', UX_ACCEPTANCE_TREE)).toEqual({
      cascaded: {
        deletedFolderIds: ['folder-work', 'folder-vms'],
        deletedConnectionIds: [],
        movedConnectionIds: ['conn-web-dev', 'conn-db-dev', 'conn-term'],
      },
      // Folder deletion preserves sessions (Req 10.16).
      stoppedSessionIds: [],
    });
  });

  it('moves nested-folder connections too', () => {
    expect(predictFolderDeletion('folder-vms', UX_ACCEPTANCE_TREE)).toEqual({
      cascaded: {
        deletedFolderIds: ['folder-vms'],
        deletedConnectionIds: [],
        movedConnectionIds: ['conn-web-dev', 'conn-db-dev'],
      },
      stoppedSessionIds: [],
    });
  });
});

describe('predictLibraryDeletion', () => {
  it('removes the folders, deletes the connections, stops their sessions', () => {
    expect(predictLibraryDeletion('lib-team', UX_ACCEPTANCE_TREE, SESSIONS)).toEqual({
      cascaded: {
        deletedFolderIds: ['folder-prod'],
        deletedConnectionIds: ['conn-web-prod', 'conn-db-prod', 'conn-bastion', 'conn-quick'],
        movedConnectionIds: [],
      },
      stoppedSessionIds: ['sess-bastion'],
    });
  });
});

describe('deletionCountsFromReport', () => {
  it('consumes a real DeletionReport shape unchanged', () => {
    const report: DeletionReport = {
      deleted: true,
      cascaded: {
        deletedFolderIds: ['f1'],
        deletedConnectionIds: ['c1', 'c2'],
        movedConnectionIds: [],
      },
      stoppedSessionIds: ['s1'],
      orphanedCredentialIds: ['c1'],
    };
    expect(deletionCountsFromReport(report)).toEqual({
      cascaded: report.cascaded,
      stoppedSessionIds: ['s1'],
    });
  });
});

// ── Exact-count confirmation text (Req 10.18) ────────────────────────

describe('buildDeletionConfirmation', () => {
  it('states the exact counts for a connection deletion', () => {
    const counts = predictConnectionDeletion('conn-bastion', SESSIONS);
    const confirmation = buildDeletionConfirmation('connection', 'bastion', counts);
    expect(confirmation.title).toBe('Delete "bastion"?');
    expect(confirmation.lines).toEqual([
      'Folders removed: 0',
      'Connections deleted: 1',
      'Connections moved to library root: 0',
      'Sessions stopped: 1',
    ]);
    expect(confirmation.text).toBe(
      'Delete "bastion"? Folders removed: 0. Connections deleted: 1. ' +
        'Connections moved to library root: 0. Sessions stopped: 1.'
    );
  });

  it('states the exact counts for a folder deletion (moved, not deleted)', () => {
    const confirmation = buildDeletionConfirmation(
      'folder',
      'Work',
      predictFolderDeletion('folder-work', UX_ACCEPTANCE_TREE)
    );
    expect(confirmation.lines).toEqual([
      'Folders removed: 2',
      'Connections deleted: 0',
      'Connections moved to library root: 3',
      'Sessions stopped: 0',
    ]);
  });

  it('states the exact counts for a library deletion', () => {
    const confirmation = buildDeletionConfirmation(
      'library',
      'Team Library',
      predictLibraryDeletion('lib-team', UX_ACCEPTANCE_TREE, SESSIONS)
    );
    expect(confirmation.lines).toEqual([
      'Folders removed: 1',
      'Connections deleted: 4',
      'Connections moved to library root: 0',
      'Sessions stopped: 1',
    ]);
  });
});

describe('buildDeletionOutcomeToast', () => {
  it('surfaces the real report counts after deletion', () => {
    const report: DeletionReport = {
      deleted: true,
      cascaded: {
        deletedFolderIds: ['folder-prod'],
        deletedConnectionIds: ['conn-web-prod', 'conn-db-prod', 'conn-bastion', 'conn-quick'],
        movedConnectionIds: [],
      },
      stoppedSessionIds: ['sess-bastion'],
      orphanedCredentialIds: [],
    };
    expect(buildDeletionOutcomeToast('Team Library', deletionCountsFromReport(report))).toBe(
      'Deleted "Team Library". Folders removed: 1. Connections deleted: 4. ' +
        'Connections moved to library root: 0. Sessions stopped: 1.'
    );
  });
});

// ── Duplication expectations (Req 10.19) ─────────────────────────────

describe('buildDuplicationNotice', () => {
  it('states the new copy, same folder and required password re-entry', () => {
    expect(buildDuplicationNotice({ id: 'conn-bastion-copy', name: 'bastion (copy)' })).toBe(
      '"bastion (copy)" created with a new ID in the same folder. ' +
        'Enter its password before connecting — credentials are never copied or shared.'
    );
  });
});
