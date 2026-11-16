import type {
  CascadedIds,
  DeletionReport,
  LibraryTree,
  NodeKind,
  SessionInfo,
} from '../types';

/**
 * Task 8.3 (Req 10.18/10.19): deletion-confirmation and duplication
 * models for the tree. The exact counts surfaced in confirmations come
 * from the same shapes the backend reports (`CascadedIds` from
 * `delete_folder`, `DeletionReport` from `delete_connection`/
 * `delete_library`, tasks 7.2/7.3); prediction mirrors those semantics
 * from live tree + session state so the pre-confirm dialog can state
 * the counts before anything is destroyed.
 */

/** The count subset of a `DeletionReport` the confirmation states (10.18). */
export interface DeletionCounts {
  cascaded: CascadedIds;
  stoppedSessionIds: string[];
}

function subtreeFolderIds(tree: LibraryTree, rootId: string): string[] {
  const ids = new Set<string>([rootId]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const f of tree.folders) {
      if (f.parentId !== null && ids.has(f.parentId) && !ids.has(f.id)) {
        ids.add(f.id);
        grew = true;
      }
    }
  }
  // Tree order (folders array order), root first.
  return tree.folders.filter((f) => ids.has(f.id)).map((f) => f.id);
}

/** Connection deletion (Req 10.17/10.18): one connection, its sessions stopped. */
export function predictConnectionDeletion(
  connectionId: string,
  sessions: Iterable<SessionInfo>
): DeletionCounts {
  return {
    cascaded: {
      deletedFolderIds: [],
      deletedConnectionIds: [connectionId],
      movedConnectionIds: [],
    },
    stoppedSessionIds: Array.from(sessions)
      .filter((s) => s.connectionId === connectionId)
      .map((s) => s.id),
  };
}

/**
 * Folder deletion (Req 10.16): the subtree folders are removed and every
 * descendant connection moves to the library root — connections are
 * never deleted and sessions are preserved.
 */
export function predictFolderDeletion(folderId: string, tree: LibraryTree): DeletionCounts {
  const folderIds = new Set(subtreeFolderIds(tree, folderId));
  return {
    cascaded: {
      deletedFolderIds: [...folderIds],
      deletedConnectionIds: [],
      movedConnectionIds: tree.connections
        .filter((c) => c.folderId !== null && folderIds.has(c.folderId))
        .map((c) => c.id),
    },
    stoppedSessionIds: [],
  };
}

/** Library deletion (Req 10.17/10.18): folders and connections gone, sessions stopped. */
export function predictLibraryDeletion(
  libraryId: string,
  tree: LibraryTree,
  sessions: Iterable<SessionInfo>
): DeletionCounts {
  const deletedConnectionIds = tree.connections
    .filter((c) => c.libraryId === libraryId)
    .map((c) => c.id);
  const deleted = new Set(deletedConnectionIds);
  return {
    cascaded: {
      deletedFolderIds: tree.folders.filter((f) => f.libraryId === libraryId).map((f) => f.id),
      deletedConnectionIds,
      movedConnectionIds: [],
    },
    stoppedSessionIds: Array.from(sessions)
      .filter((s) => deleted.has(s.connectionId))
      .map((s) => s.id),
  };
}

/** View the real backend report as confirmation counts (10.18 exact data). */
export function deletionCountsFromReport(report: DeletionReport): DeletionCounts {
  return { cascaded: report.cascaded, stoppedSessionIds: [...report.stoppedSessionIds] };
}

export interface DeletionConfirmation {
  title: string;
  /** Exact-count lines: folders removed, connections deleted/moved, sessions stopped. */
  lines: string[];
  /** Title + lines as one sentence. */
  text: string;
}

/**
 * Confirmation text stating the exact counts (Req 10.18) for a node of
 * the given kind about to be deleted.
 */
export function buildDeletionConfirmation(
  _kind: NodeKind,
  name: string,
  counts: DeletionCounts
): DeletionConfirmation {
  const title = `Delete "${name}"?`;
  const lines = [
    `Folders removed: ${counts.cascaded.deletedFolderIds.length}`,
    `Connections deleted: ${counts.cascaded.deletedConnectionIds.length}`,
    `Connections moved to library root: ${counts.cascaded.movedConnectionIds.length}`,
    `Sessions stopped: ${counts.stoppedSessionIds.length}`,
  ];
  return { title, lines, text: [title, ...lines.map((l) => `${l}.`)].join(' ') };
}

/** Post-deletion toast stating the counts from the real report. */
export function buildDeletionOutcomeToast(name: string, counts: DeletionCounts): string {
  return [
    `Deleted "${name}".`,
    `Folders removed: ${counts.cascaded.deletedFolderIds.length}.`,
    `Connections deleted: ${counts.cascaded.deletedConnectionIds.length}.`,
    `Connections moved to library root: ${counts.cascaded.movedConnectionIds.length}.`,
    `Sessions stopped: ${counts.stoppedSessionIds.length}.`,
  ].join(' ');
}

/**
 * Duplication expectations (Req 10.19): a fresh ID in the same folder
 * with non-secret settings, and the password must be re-entered because
 * the original credential is never copied or shared.
 */
export function buildDuplicationNotice(copy: { id: string; name: string }): string {
  return `"${copy.name}" created with a new ID in the same folder. Enter its password before connecting — credentials are never copied or shared.`;
}
