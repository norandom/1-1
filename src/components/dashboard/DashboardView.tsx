import { useEffect, useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import { Plus, Monitor, Loader2 } from 'lucide-react';
import type { ConnectionSummary, NodeId } from '../../types';
import { buildDashboardModel, fetchDescendants } from '../../lib/dashboard-view';
import { statusesFromSessions } from '../../lib/tree-a11y';
import { useLibraryStore, selectNewConnectionDestination, selectIsWriteBlocked } from '../../stores/libraryStore';
import { useSessionStore } from '../../stores/sessionStore';
import { useConnectionStore } from '../../stores/connectionStore';
import { useConnectAction } from '../../hooks/useConnectAction';
import { ConnectionForm } from '../connections/ConnectionForm';
import { DashboardCard } from './DashboardCard';
import { Button } from '@/components/ui/button';
import { RECOVERY_BLOCKED_MESSAGE } from '../../lib/recovery';

/**
 * Task 8.5 (Req 10.5/10.6/11.6, design.md L395): the dashboard of the
 * selected library/folder. The listing comes from the backend
 * `list_descendants` wrapper (nested folders included); live status is
 * joined from `sessionStore` (text, never color-only). Double-click and
 * the Connect button use the ONE shared connect action. "New Connection"
 * is scoped to the selected node via the 8.2 destination selector — the
 * existing ConnectionForm files into it. Navigating here never disturbs
 * running sessions (pinned by DASHBOARD_WITH_TABS_PINS; tabs are 8.6).
 */
export function DashboardView({ node }: { node: NodeId }) {
  const tree = useLibraryStore((s) => s.tree);
  const sessions = useSessionStore((s) => s.sessions);
  const connections = useConnectionStore((s) => s.connections);
  // Raw slice + useMemo (NOT a direct derived selector: unstable snapshots
  // loop useSyncExternalStore — see ConnectionTree for the full note).
  const selectedNodeId = useLibraryStore((s) => s.selectedNodeId);
  const destination = useMemo(
    () => selectNewConnectionDestination({ tree, selectedNodeId }),
    [tree, selectedNodeId]
  );
  // Task 8.7 (Req 10.12): the create CTAs are disabled while the store
  // is write-blocked — the dashboard itself stays viewable.
  const mutationsBlocked = useLibraryStore(selectIsWriteBlocked);
  const { connect } = useConnectAction();

  const [summaries, setSummaries] = useState<ConnectionSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showNewConnection, setShowNewConnection] = useState(false);

  // Refresh the backend listing after tree mutations as well as selection.
  // Cancel stale responses when a move or save changes membership again.
  useEffect(() => {
    let cancelled = false;
    fetchDescendants(node)
      .then((result) => {
        if (!cancelled) {
          setSummaries(result);
          setError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [node, tree]);

  const statuses = useMemo(() => statusesFromSessions(sessions.values()), [sessions]);
  const model = buildDashboardModel({
    tree,
    selectedNodeId: node,
    statuses,
    summaries: summaries ?? [],
  });
  const detailsById = useMemo(
    () => new Map(connections.map((c) => [c.id, c] as const)),
    [connections]
  );

  if (!model) return null; // stale selection — MainPanel switches views

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="shrink-0 px-4 pb-4 pt-6 md:px-8">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
          <div className="min-w-0">
            <h1 className="truncate text-2xl font-bold tracking-tight text-foreground">
              {model.title}
            </h1>
            <p className="mt-1 flex items-center gap-2 text-sm text-muted-foreground">
              <span>{model.statusText}</span>
              <span aria-hidden="true">·</span>
              <span>
                {model.cards.length} connection{model.cards.length !== 1 ? 's' : ''}
              </span>
            </p>
          </div>
          <Button
            onClick={() => setShowNewConnection(true)}
            disabled={mutationsBlocked}
            title={
              mutationsBlocked
                ? RECOVERY_BLOCKED_MESSAGE
                : destination
                  ? `New connection in ${destination.label}`
                  : undefined
            }
          >
            <Plus className="size-4" strokeWidth={2.5} />
            New Connection
          </Button>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-4 pb-8 md:px-8">
        {error ? (
          <div className="flex h-64 flex-col items-center justify-center text-center">
            <p className="text-base font-semibold text-foreground">Could not load connections</p>
            <p className="mt-1 max-w-[280px] text-sm text-muted-foreground">{error}</p>
          </div>
        ) : summaries === null ? (
          <div className="flex h-64 items-center justify-center">
            <Loader2 className="size-8 animate-spin text-primary" />
          </div>
        ) : model.emptyState ? (
          <motion.div
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex h-64 flex-col items-center justify-center text-center"
          >
            <div className="mb-5 flex size-16 items-center justify-center rounded-2xl bg-muted">
              <Monitor className="size-7 text-muted-foreground" />
            </div>
            <h3 className="mb-1.5 text-base font-semibold text-foreground">No connections yet</h3>
            <p className="mb-5 max-w-[280px] text-sm text-muted-foreground">
              {destination
                ? `Add a connection to ${destination.label}`
                : 'Add your first remote desktop connection to get started'}
            </p>
            <Button onClick={() => setShowNewConnection(true)} disabled={mutationsBlocked}>
              <Plus className="size-4" />
              Add Connection
            </Button>
          </motion.div>
        ) : (
          <div
            className="grid gap-5"
            style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(100%, 300px), 1fr))' }}
          >
            {model.cards.map((card) => (
              <DashboardCard
                key={card.connectionId}
                card={card}
                detail={detailsById.get(card.connectionId)}
                onConnect={connect}
              />
            ))}
          </div>
        )}
      </div>

      {/* New Connection, scoped to the selected destination (8.2) */}
      {showNewConnection && (
        <ConnectionForm connection={null} onClose={() => setShowNewConnection(false)} />
      )}
    </div>
  );
}
