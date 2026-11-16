import { useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from '@dnd-kit/core';
import type { DragEndEvent } from '@dnd-kit/core';
import {
  ChevronDown,
  ChevronRight,
  Folder,
  FolderOpen,
  Library as LibraryIcon,
  Monitor,
  MoreVertical,
} from 'lucide-react';
import type { ConnectionConfig, NodeId, SessionStatus } from '../../types';
import {
  FILTER_EMPTY_STATE_TEXT,
  selectEffectiveExpansion,
  selectFilterResult,
  selectVisibleNodes,
  useLibraryStore,
} from '../../stores/libraryStore';
import { useSessionStore } from '../../stores/sessionStore';
import { useConnectionStore } from '../../stores/connectionStore';
import { useToastStore } from '../../stores/toastStore';
import * as tauri from '../../lib/tauri';
import { applyTreeKey } from '../../lib/tree-keyboard';
import type { TreeA11yNode, TreeKey } from '../../lib/ux-tree-states';
import {
  buildTreeA11yNodes,
  statusesFromSessions,
  treeItemAria,
  treeItemLabel,
} from '../../lib/tree-a11y';
import { contextMenuItems, menuActionFor } from '../../lib/tree-menu';
import { dragEventToMove, moveDestinations, moveTreeNodes } from '../../lib/tree-move';
import type { MoveOutcome, MoveTrigger } from '../../lib/tree-move';
import type { DeletionCounts } from '../../lib/tree-confirm';
import {
  buildDeletionConfirmation,
  buildDeletionOutcomeToast,
  buildDuplicationNotice,
  deletionCountsFromReport,
  predictConnectionDeletion,
  predictFolderDeletion,
  predictLibraryDeletion,
} from '../../lib/tree-confirm';
import { ConnectionForm } from '../connections/ConnectionForm';
// Task 8.5 (Req 10.6): the ONE shared connect action (design.md L379).
import { useConnectAction } from '../../hooks/useConnectAction';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

/**
 * Task 8.3 (Req 10.3/10.4/10.7/10.13/10.14/10.18/10.19): the ConnectionTree
 * render layer. Deliberately thin — every keyboard/focus/menu/status/
 * confirmation decision lives in the tested pure modules (tree-keyboard,
 * tree-a11y, tree-menu, tree-confirm, tree-rail) and this component only
 * projects their results onto Radix primitives. Task 8.4 (Req 10.8/10.14/
 * 10.15) adds movement: dnd-kit drag (TreeRow) and the keyboard Move
 * dialog both funnel through the one shared action in tree-move.
 * Rendered behavior is verified by the group-12 E2E tier per the design
 * testing strategy.
 */

const nodeKey = (n: NodeId): string => `${n.kind}:${n.id}`;

const sameNode = (a: NodeId | null, b: NodeId | null): boolean =>
  a !== null && b !== null && a.kind === b.kind && a.id === b.id;

/** Dot color is decorative only — the textual status is always present (10.14). */
const STATUS_DOT_COLOR: Record<SessionStatus, string> = {
  connected: 'var(--color-success)',
  connecting: 'var(--color-warning)',
  reconnecting: 'var(--color-warning)',
  error: 'var(--color-danger)',
  disconnected: 'var(--color-text-muted)',
};

interface TreeRowProps {
  a11y: TreeA11yNode;
  /** Live session status — connections only; null for containers. */
  status: SessionStatus | null;
  isFilterActive: boolean;
  isFocused: boolean;
  menuOpen: boolean;
  registerRef: (key: string, el: HTMLDivElement | null) => void;
  focusRow: (key: string) => void;
  onRowClick: (node: NodeId, key: string) => void;
  onConnect: (node: NodeId) => void;
  onOpenMenu: (node: NodeId) => void;
  onMenuOpenChange: (open: boolean, node: NodeId) => void;
  onMenuItemSelect: (node: NodeId, item: string) => void;
  onToggleExpanded: (id: string) => void;
}

/**
 * One tree row (Req 10.3/10.13/10.14) and its drag affordance (10.8):
 * connections and folders are dnd-kit draggables, folders and libraries
 * are droppables — both feed the ONE shared move action in tree-move.
 * Libraries are neither draggable source nor movable (10.7); the pinned
 * treeitem aria contract is kept (dnd-kit only adds roledescription +
 * instructions; Space lifts via the keyboard sensor, Enter stays the
 * tree's connect/select key per TREE_KEYBOARD_RULES).
 */
function TreeRow({
  a11y,
  status,
  isFilterActive,
  isFocused,
  menuOpen,
  registerRef,
  focusRow,
  onRowClick,
  onConnect,
  onOpenMenu,
  onMenuOpenChange,
  onMenuItemSelect,
  onToggleExpanded,
}: TreeRowProps) {
  const key = nodeKey(a11y.node);
  const isContainer = a11y.kind !== 'connection';
  const canDrag = a11y.kind !== 'library';
  const canDrop = isContainer;

  // Both hooks run unconditionally (rules of hooks); `disabled` keeps
  // libraries from dragging and connections from receiving drops.
  const { attributes, listeners, setNodeRef: setDragRef, isDragging, transform } = useDraggable({
    id: key,
    disabled: !canDrag,
  });
  const { setNodeRef: setDropRef, isOver } = useDroppable({ id: key, disabled: !canDrop });

  const setRefs = (el: HTMLDivElement | null): void => {
    registerRef(key, el);
    setDragRef(el);
    setDropRef(el);
  };

  return (
    <div
      ref={setRefs}
      role="treeitem"
      aria-label={treeItemLabel(a11y)}
      {...treeItemAria(a11y)}
      {...(canDrag
        ? {
            'aria-roledescription': attributes['aria-roledescription'],
            'aria-describedby': attributes['aria-describedby'],
          }
        : {})}
      {...(canDrag ? listeners : {})}
      tabIndex={isFocused ? 0 : -1}
      onClick={() => onRowClick(a11y.node, key)}
      onDoubleClick={() => {
        if (a11y.kind === 'connection') onConnect(a11y.node);
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onOpenMenu(a11y.node);
      }}
      className={cn(
        'group flex cursor-pointer items-center gap-1.5 rounded-md py-1.5 pr-1 text-[13px] outline-none',
        'focus-visible:ring-2 focus-visible:ring-[var(--color-accent)]',
        a11y.selected && 'bg-[var(--color-accent)]/10 text-[var(--color-accent)]',
        isOver && canDrop && 'ring-1 ring-[var(--color-accent)]',
        isDragging && 'opacity-50'
      )}
      style={{
        paddingLeft: (a11y.level - 1) * 14 + 6,
        transform: transform ? `translate3d(${transform.x}px, ${transform.y}px, 0)` : undefined,
      }}
    >
      {isContainer && !isFilterActive && (
        <button
          type="button"
          tabIndex={-1}
          aria-hidden="true"
          onClick={(e) => {
            e.stopPropagation();
            onToggleExpanded(a11y.node.id);
          }}
          onPointerDown={(e) => e.stopPropagation()}
          className="shrink-0 p-0.5 text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]"
        >
          {a11y.expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </button>
      )}
      {isContainer && isFilterActive && <span className="w-[19px] shrink-0" />}
      <span className="shrink-0 text-[var(--color-text-muted)]">
        {a11y.kind === 'library' ? (
          <LibraryIcon size={15} />
        ) : a11y.kind === 'folder' ? (
          a11y.expanded ? <FolderOpen size={15} /> : <Folder size={15} />
        ) : (
          <Monitor size={15} />
        )}
      </span>
      <span className="flex-1 truncate">{a11y.name}</span>
      <span className="flex shrink-0 items-center gap-1 text-[10px] text-[var(--color-text-muted)]">
        {status && (
          <span
            aria-hidden="true"
            className="inline-block h-1.5 w-1.5 rounded-full"
            style={{ background: STATUS_DOT_COLOR[status] }}
          />
        )}
        {a11y.statusText}
      </span>
      <DropdownMenu
        open={menuOpen}
        onOpenChange={(open) => onMenuOpenChange(open, a11y.node)}
      >
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            tabIndex={-1}
            aria-label={`Actions for ${a11y.name}`}
            onClick={(e) => e.stopPropagation()}
            onPointerDown={(e) => e.stopPropagation()}
            className="shrink-0 rounded p-1 text-[var(--color-text-muted)] opacity-0 focus-visible:opacity-100 group-hover:opacity-100 hover:text-[var(--color-text-secondary)]"
          >
            <MoreVertical size={14} />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="start"
          onCloseAutoFocus={(e) => {
            // Pinned (ux-acceptance Row 1): closing the menu —
            // dismissed or after selecting — returns focus to the node.
            e.preventDefault();
            focusRow(key);
          }}
        >
          {contextMenuItems(a11y.kind).map((item) => (
            <DropdownMenuItem
              key={item}
              onSelect={() => {
                onMenuItemSelect(a11y.node, item);
              }}
            >
              {item}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

export function ConnectionTree() {
  const navigate = useNavigate();
  const location = useLocation();

  // Tree state (8.2 store): tree/selection/expansion persisted, filter.
  const tree = useLibraryStore((s) => s.tree);
  const selectedNodeId = useLibraryStore((s) => s.selectedNodeId);
  const filter = useLibraryStore((s) => s.filter);
  const isLoading = useLibraryStore((s) => s.isLoading);
  const error = useLibraryStore((s) => s.error);
  const selectNode = useLibraryStore((s) => s.selectNode);
  const toggleExpanded = useLibraryStore((s) => s.toggleExpanded);
  const setFilter = useLibraryStore((s) => s.setFilter);
  const fetchTree = useLibraryStore((s) => s.fetchTree);

  // Live session statuses (design.md: the tree subscribes sessionStore).
  const sessions = useSessionStore((s) => s.sessions);
  const removeSession = useSessionStore((s) => s.removeSession);

  const connections = useConnectionStore((s) => s.connections);
  const fetchConnections = useConnectionStore((s) => s.fetchConnections);
  const duplicateConnection = useConnectionStore((s) => s.duplicateConnection);
  const addToast = useToastStore((s) => s.addToast);

  const visible = useLibraryStore(selectVisibleNodes);
  const effectiveExpansion = useLibraryStore(selectEffectiveExpansion);
  const filterResult = useLibraryStore(selectFilterResult);
  const isFilterActive = filterResult !== null;

  const statuses = useMemo(() => statusesFromSessions(sessions.values()), [sessions]);
  const a11yNodes = useMemo(
    () => buildTreeA11yNodes({ tree, statuses, expanded: effectiveExpansion, selectedNodeId }),
    [tree, statuses, effectiveExpansion, selectedNodeId]
  );

  // ── Roving focus (exactly one tabbable treeitem, Req 10.13) ────────
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const itemRefs = useRef(new Map<string, HTMLDivElement>());

  const visibleKeys = useMemo(() => new Set(visible.map(nodeKey)), [visible]);
  // Re-anchor when the focused node vanished (filter, deletion, move).
  const effectiveFocusedKey =
    focusedKey !== null && visibleKeys.has(focusedKey)
      ? focusedKey
      : selectedNodeId && visibleKeys.has(nodeKey(selectedNodeId))
        ? nodeKey(selectedNodeId)
        : visible[0]
          ? nodeKey(visible[0])
          : null;

  const focusNode = (node: NodeId): void => {
    const key = nodeKey(node);
    setFocusedKey(key);
    requestAnimationFrame(() => itemRefs.current.get(key)?.focus());
  };

  const registerItemRef = (key: string, el: HTMLDivElement | null): void => {
    if (el) itemRefs.current.set(key, el);
    else itemRefs.current.delete(key);
  };

  // ── Actions (10.6 connect path today: the session view; tabs are 8.6) ──
  // Task 8.5: connect goes through the ONE shared action — the dashboard
  // double-click and quick connect use exactly this entry too.
  const { connect: connectNode } = useConnectAction();

  /**
   * Req 10.5/11.6: selecting a library/folder shows its dashboard. The
   * selection drives the `/` main panel; navigating there replaces a
   * session view WITHOUT disturbing the session (SessionView has no
   * unmount disconnect; 7.1 admission reconnects it on return).
   */
  const showDashboard = (): void => {
    if (location.pathname !== '/') navigate('/');
  };

  const nodeName = (node: NodeId): string => {
    if (node.kind === 'library') return tree.libraries.find((l) => l.id === node.id)?.name ?? node.id;
    if (node.kind === 'folder') return tree.folders.find((f) => f.id === node.id)?.name ?? node.id;
    return tree.connections.find((c) => c.id === node.id)?.name ?? node.id;
  };

  const duplicate = async (node: NodeId): Promise<void> => {
    try {
      // 10.19: backend copies non-secret settings into the same folder with
      // a fresh id; the toast surfaces the password re-entry expectation.
      const copy = await duplicateConnection(node.id);
      await fetchTree();
      selectNode({ kind: 'connection', id: copy.id });
      addToast({
        message: buildDuplicationNotice({ id: copy.id, name: copy.name }),
        type: 'info',
        duration: 8000,
      });
    } catch (e) {
      addToast({ message: e instanceof Error ? e.message : 'Failed to duplicate connection', type: 'error' });
    }
  };

  // ── Context menu (Shift+F10 / right-click; focus contract pinned) ──
  const [menuNode, setMenuNode] = useState<NodeId | null>(null);

  const openMenu = (node: NodeId): void => {
    setFocusedKey(nodeKey(node));
    setMenuNode(node);
  };

  const [editingConnection, setEditingConnection] = useState<ConnectionConfig | null>(null);
  const [showNewConnection, setShowNewConnection] = useState(false);
  const [renameNode, setRenameNode] = useState<NodeId | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [newFolderParent, setNewFolderParent] = useState<NodeId | null>(null);
  const [newFolderName, setNewFolderName] = useState('');
  const [deleteNode, setDeleteNode] = useState<NodeId | null>(null);
  const [moveActor, setMoveActor] = useState<NodeId | null>(null);
  const [moveError, setMoveError] = useState<string | null>(null);

  const runMenuAction = (node: NodeId, item: string): void => {
    switch (menuActionFor(item)) {
      case 'connect':
        connectNode(node);
        break;
      case 'edit': {
        const conn = connections.find((c) => c.id === node.id);
        if (conn) setEditingConnection(conn);
        break;
      }
      case 'duplicate':
        void duplicate(node);
        break;
      case 'move':
        setMoveError(null);
        setMoveActor(node);
        break;
      case 'delete':
        setDeleteNode(node);
        break;
      case 'new-connection':
        // 8.2 destination matrix: the form files into the selected node.
        selectNode(node);
        setShowNewConnection(true);
        break;
      case 'new-folder':
        setNewFolderName('');
        setNewFolderParent(node);
        break;
      case 'rename':
        setRenameValue(nodeName(node));
        setRenameNode(node);
        break;
    }
  };

  // ── Keyboard controller wiring (all semantics in tree-keyboard) ────
  const handleTreeKeyDown = (event: React.KeyboardEvent<HTMLDivElement>): void => {
    if (!effectiveFocusedKey) return;
    const focused = a11yNodes.find((a) => nodeKey(a.node) === effectiveFocusedKey)?.node;
    if (!focused) return;

    let key: TreeKey | null = null;
    if (
      event.key === 'ArrowUp' || event.key === 'ArrowDown' ||
      event.key === 'ArrowLeft' || event.key === 'ArrowRight' ||
      event.key === 'Home' || event.key === 'End'
    ) {
      key = event.key;
    } else if (event.key === 'Enter') {
      key = 'Enter';
    } else if (event.key === 'F10' && event.shiftKey) {
      key = 'Shift+F10';
    }
    if (!key) return;
    event.preventDefault();

    const result = applyTreeKey({ key, focus: focused, tree, expanded: effectiveExpansion });
    // Expansion changes mutate the saved map — a no-op while filtering
    // (8.2 guard; expansion is temporary then and no affordance renders).
    if (result.expansionChange) toggleExpanded(result.expansionChange.nodeId);
    if (nodeKey(result.focus) !== effectiveFocusedKey) focusNode(result.focus);

    if (result.action?.type === 'select-dashboard') {
      // Selection is store-owned; the dashboard panel itself is task 8.5.
      selectNode(result.action.node);
      showDashboard();
    } else if (result.action?.type === 'connect') {
      connectNode(result.action.node);
    } else if (result.action?.type === 'open-context-menu') {
      openMenu(result.action.node);
    }
  };

  // ── Deletion confirmation (10.17/10.18: exact counts, real reports) ──
  const deleteCounts = useMemo<DeletionCounts | null>(() => {
    if (!deleteNode) return null;
    if (deleteNode.kind === 'connection') return predictConnectionDeletion(deleteNode.id, sessions.values());
    if (deleteNode.kind === 'folder') return predictFolderDeletion(deleteNode.id, tree);
    return predictLibraryDeletion(deleteNode.id, tree, sessions.values());
  }, [deleteNode, tree, sessions]);

  const deleteConfirmation =
    deleteNode && deleteCounts
      ? buildDeletionConfirmation(deleteNode.kind, nodeName(deleteNode), deleteCounts)
      : null;

  const confirmDelete = async (): Promise<void> => {
    if (!deleteNode) return;
    const node = deleteNode;
    const name = nodeName(node);
    try {
      let counts: DeletionCounts;
      if (node.kind === 'connection') {
        const report = await tauri.deleteConnection(node.id);
        counts = deletionCountsFromReport(report);
        for (const sessionId of report.stoppedSessionIds) removeSession(sessionId);
      } else if (node.kind === 'folder') {
        const cascaded = await tauri.deleteFolder(node.id);
        counts = { cascaded, stoppedSessionIds: [] };
      } else {
        const report = await tauri.deleteLibrary(node.id);
        counts = deletionCountsFromReport(report);
        for (const sessionId of report.stoppedSessionIds) removeSession(sessionId);
      }
      await Promise.all([fetchTree(), fetchConnections()]);
      addToast({ message: buildDeletionOutcomeToast(name, counts), type: 'success' });
    } catch (e) {
      // 10.17: keep showing the backend's data — resync and report.
      await fetchTree();
      addToast({ message: e instanceof Error ? e.message : 'Delete failed', type: 'error' });
    } finally {
      setDeleteNode(null);
    }
  };

  // ── Move: ONE shared action for both triggers (task 8.4, 10.8/10.14/ ──
  // 10.15). The dialog consumes the shared destination filtering; drag
  // and keyboard both funnel through `moveTreeNodes`, whose outcome pins
  // where focus goes (follows the node; restored on failure).
  const dialogDestinations = useMemo(() => (moveActor ? moveDestinations(tree, moveActor) : []), [moveActor, tree]);

  /**
   * The shared move runner: selection + focus follow `outcome.focusAfter`
   * in BOTH cases — after a persisted move the node sits under its
   * destination, after a rejected/failed one the rollback kept it in its
   * previous location, so focus returns exactly there (MOVE_STATE_PINS).
   */
  const runSharedMove = async (
    trigger: MoveTrigger,
    source: NodeId,
    target: NodeId
  ): Promise<MoveOutcome> => {
    const { tree: currentTree, moveNode } = useLibraryStore.getState();
    const outcome = await moveTreeNodes({ tree: currentTree, moveNode }, trigger, source, target);
    selectNode(outcome.focusAfter);
    focusNode(outcome.focusAfter);
    return outcome;
  };

  const doMove = async (destination: NodeId): Promise<void> => {
    if (!moveActor) return;
    const outcome = await runSharedMove('keyboard-move-dialog', moveActor, destination);
    if (outcome.status === 'moved') {
      setMoveActor(null);
      setMoveError(null);
    } else {
      // 10.15: dialog stays open on the restored tree with the message.
      setMoveError(outcome.message ?? 'Move failed.');
    }
  };

  // ── Drag-and-drop (10.8): dnd-kit events -> (source, target) pairs ──
  // -> the same shared action. Space-only keyboard sensor activation so
  // the pinned Enter behavior (connect/select) is untouched.
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, {
      keyboardCodes: { start: ['Space'], cancel: ['Escape'], end: ['Space'] },
    })
  );

  const handleDragEnd = (event: DragEndEvent): void => {
    const pair = dragEventToMove({
      activeKey: String(event.active.id),
      overKey: event.over ? String(event.over.id) : null,
    });
    if (!pair) return;
    void (async () => {
      const outcome = await runSharedMove('drag-drop', pair.source, pair.target);
      if (outcome.status === 'restored') {
        // 10.15 for the drag surface: toast the error; location/focus
        // are already restored by the rollback + focusAfter above.
        addToast({ message: outcome.message ?? 'Move failed.', type: 'error' });
      }
    })();
  };

  const confirmRename = async (): Promise<void> => {
    if (!renameNode) return;
    const name = renameValue.trim();
    if (!name) return;
    try {
      if (renameNode.kind === 'library') await tauri.renameLibrary(renameNode.id, name);
      else if (renameNode.kind === 'folder') await tauri.renameFolder(renameNode.id, name);
      await fetchTree();
      setRenameNode(null);
    } catch (e) {
      addToast({ message: e instanceof Error ? e.message : 'Rename failed', type: 'error' });
    }
  };

  const confirmNewFolder = async (): Promise<void> => {
    if (!newFolderParent) return;
    const name = newFolderName.trim();
    if (!name) return;
    const parentFolder =
      newFolderParent.kind === 'folder'
        ? tree.folders.find((f) => f.id === newFolderParent.id)
        : undefined;
    const libraryId = newFolderParent.kind === 'library' ? newFolderParent.id : parentFolder?.libraryId;
    if (!libraryId) return;
    const parentId = newFolderParent.kind === 'folder' ? parentFolder!.id : null;
    try {
      await tauri.createFolder(libraryId, parentId, name);
      await fetchTree();
      setNewFolderParent(null);
    } catch (e) {
      addToast({ message: e instanceof Error ? e.message : 'Failed to create folder', type: 'error' });
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* Inline filter (Req 10.9) */}
      <div className="shrink-0 px-3 pb-2">
        <Input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter connections"
          aria-label="Filter connections"
          className="h-8 text-[13px]"
        />
      </div>

      {/* The tree (Req 10.3/10.13/10.14) + drag-and-drop (10.8, 8.4):
          every drop funnels into the same shared action as the Move
          dialog — see tree-move and runSharedMove above. */}
      <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
        <div
          role="tree"
          aria-label="Connections"
          onKeyDown={handleTreeKeyDown}
          className="min-h-0 flex-1 overflow-y-auto px-2 pb-2"
        >
          {isLoading && tree.libraries.length === 0 ? (
            <p className="px-2 py-4 text-[12px] text-[var(--color-text-muted)]">Loading…</p>
          ) : error ? (
            <p role="alert" className="px-2 py-4 text-[12px] text-[var(--color-danger)]">{error}</p>
          ) : isFilterActive && a11yNodes.length === 0 ? (
            <p className="px-2 py-4 text-[12px] text-[var(--color-text-muted)]">{FILTER_EMPTY_STATE_TEXT}</p>
          ) : a11yNodes.length === 0 ? (
            <p className="px-2 py-4 text-[12px] text-[var(--color-text-muted)]">No libraries yet</p>
          ) : (
            a11yNodes.map((a11y) => (
              <TreeRow
                key={nodeKey(a11y.node)}
                a11y={a11y}
                status={a11y.kind === 'connection' ? (statuses[a11y.node.id] ?? 'disconnected') : null}
                isFilterActive={isFilterActive}
                isFocused={nodeKey(a11y.node) === effectiveFocusedKey}
                menuOpen={sameNode(menuNode, a11y.node)}
                registerRef={registerItemRef}
                focusRow={(key) => itemRefs.current.get(key)?.focus()}
                onRowClick={(node, key) => {
                  selectNode(node);
                  setFocusedKey(key);
                  // Single-click library/folder → its dashboard (10.5).
                  if (node.kind !== 'connection') showDashboard();
                }}
                onConnect={connectNode}
                onOpenMenu={openMenu}
                onMenuOpenChange={(open, node) => {
                  if (open) setMenuNode(node);
                  else if (sameNode(menuNode, node)) setMenuNode(null);
                }}
                onMenuItemSelect={(node, item) => {
                  setMenuNode(null);
                  runMenuAction(node, item);
                }}
                onToggleExpanded={toggleExpanded}
              />
            ))
          )}
        </div>
      </DndContext>

      {/* Deletion confirmation (10.18: exact counts) */}
      <Dialog
        open={deleteNode !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteNode(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>{deleteConfirmation?.title}</DialogTitle>
          </DialogHeader>
          <ul className="list-disc pl-5 text-[13px] text-[var(--color-text-secondary)]">
            {deleteConfirmation?.lines.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={() => setDeleteNode(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => {
                void confirmDelete();
              }}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Move (10.14 keyboard equivalent): the dialog and drag share the
          tree-move transaction; targets come from the shared filtering. */}
      <Dialog
        open={moveActor !== null}
        onOpenChange={(open) => {
          if (!open) {
            setMoveActor(null);
            setMoveError(null);
          }
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>Move {moveActor ? `"${nodeName(moveActor)}"` : ''}</DialogTitle>
          </DialogHeader>
          <p className="text-[12px] text-[var(--color-text-muted)]">
            Choose a destination folder or library root.
          </p>
          <div className="flex max-h-64 flex-col gap-1 overflow-y-auto">
            {dialogDestinations.map((dest) => (
              <Button
                key={nodeKey(dest.node)}
                variant="ghost"
                size="sm"
                className="justify-start"
                onClick={() => {
                  void doMove(dest.node);
                }}
              >
                {dest.label}
              </Button>
            ))}
          </div>
          {moveError && (
            <p role="alert" className="text-[12px] text-[var(--color-danger)]">
              {moveError}
            </p>
          )}
          <DialogFooter>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setMoveActor(null);
                setMoveError(null);
              }}
            >
              Cancel
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Rename (folders/libraries, 10.7) */}
      <Dialog
        open={renameNode !== null}
        onOpenChange={(open) => {
          if (!open) setRenameNode(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>Rename {renameNode ? nodeName(renameNode) : ''}</DialogTitle>
          </DialogHeader>
          <Input
            value={renameValue}
            onChange={(e) => setRenameValue(e.target.value)}
            aria-label="New name"
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                void confirmRename();
              }
            }}
          />
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={() => setRenameNode(null)}>
              Cancel
            </Button>
            <Button
              size="sm"
              disabled={!renameValue.trim()}
              onClick={() => {
                void confirmRename();
              }}
            >
              Rename
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* New Folder (10.7) */}
      <Dialog
        open={newFolderParent !== null}
        onOpenChange={(open) => {
          if (!open) setNewFolderParent(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>
              New Folder in {newFolderParent ? nodeName(newFolderParent) : ''}
            </DialogTitle>
          </DialogHeader>
          <Input
            value={newFolderName}
            onChange={(e) => setNewFolderName(e.target.value)}
            placeholder="Folder name"
            aria-label="Folder name"
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                void confirmNewFolder();
              }
            }}
          />
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={() => setNewFolderParent(null)}>
              Cancel
            </Button>
            <Button
              size="sm"
              disabled={!newFolderName.trim()}
              onClick={() => {
                void confirmNewFolder();
              }}
            >
              Create
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Edit / New Connection (the existing form; destination from selection) */}
      {(editingConnection !== null || showNewConnection) && (
        <ConnectionForm
          connection={editingConnection}
          onClose={() => {
            setEditingConnection(null);
            setShowNewConnection(false);
          }}
        />
      )}
    </div>
  );
}
