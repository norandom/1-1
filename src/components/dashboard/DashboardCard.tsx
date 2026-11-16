import { Play, Clock } from 'lucide-react';
import type { ConnectionConfig, NodeId } from '../../types';
import type { DashboardCardModel } from '../../lib/dashboard-view';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';

/**
 * Task 8.5 (Req 10.5/10.6): the DashboardCard — a status-first
 * adaptation of ConnectionCard (design.md L395). Differences pinned by
 * the design: live session status leads (text, never color-only —
 * 10.4), and there is NO latency-on-mount probing or reachability
 * testing; connecting happens through the shared connect action on
 * double-click or the Connect button.
 */

const STATUS_COLORS: Record<DashboardCardModel['status'], string> = {
  connected: 'var(--color-success)',
  connecting: 'var(--color-warning)',
  reconnecting: 'var(--color-warning)',
  error: 'var(--color-danger)',
  disconnected: 'var(--color-text-muted)',
};

const DEFAULT_ACCENT = '#6366f1';

interface Props {
  card: DashboardCardModel;
  /** Full config when the connectionStore join hits (tags, port, …). */
  detail?: ConnectionConfig;
  onConnect: (node: NodeId) => void;
}

export function DashboardCard({ card, detail, onConnect }: Props) {
  const connectionNode: NodeId = { kind: 'connection', id: card.connectionId };

  return (
    <div
      data-testid={`dashboard-card-${card.connectionId}`}
      onDoubleClick={() => onConnect(connectionNode)}
      className="card-shadow group relative cursor-pointer overflow-hidden rounded-xl bg-card text-card-foreground ring-1 ring-foreground/10 transition-all duration-200 hover:ring-foreground/20"
    >
      {/* Color accent strip */}
      <div
        className="h-[3px] w-full"
        style={{ background: detail?.colorAccent || DEFAULT_ACCENT }}
      />

      <div className="min-w-0 p-5">
        {/* Header: name + live status */}
        <div className="mb-4 flex items-start justify-between gap-2">
          <div className="flex min-w-0 flex-1 items-center gap-3">
            <span
              aria-hidden="true"
              className={cn(
                'size-2.5 shrink-0 rounded-full transition-colors',
                card.status === 'connected' && 'pulse-online'
              )}
              style={{ background: STATUS_COLORS[card.status] }}
            />
            <h3 className="truncate text-[15px] font-semibold text-[var(--color-text-primary)]">
              {card.name}
            </h3>
          </div>
          {/* Textual live status — never color-only (10.4) */}
          <span
            className="shrink-0 text-[11px] font-medium"
            style={{ color: STATUS_COLORS[card.status] }}
          >
            {card.statusText}
          </span>
        </div>

        {/* Connection info */}
        <div className="mb-4 space-y-1.5">
          <p className="break-all font-mono text-[13px] tracking-tight text-[var(--color-text-secondary)]">
            {card.host}
            {detail ? `:${detail.port}` : ''}
          </p>
          {detail && (detail.domain || detail.username) && (
            <p className="truncate text-xs text-[var(--color-text-muted)]">
              {detail.domain ? `${detail.domain}\\` : ''}
              {detail.username}
            </p>
          )}
        </div>

        {/* Tags */}
        {detail && detail.tags.length > 0 && (
          <div className="mb-4 flex flex-wrap gap-1.5">
            {detail.tags.map((tag) => (
              <Badge key={tag} variant="secondary" className="text-[11px] font-medium">
                {tag}
              </Badge>
            ))}
          </div>
        )}

        {/* Footer */}
        <div className="flex flex-col gap-3 border-t border-[var(--color-border)] pt-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 items-center gap-1.5 text-[11px] text-[var(--color-text-muted)]">
            <Clock size={12} />
            {/* Deterministic per input (no Date.now() during render). */}
            <span className="truncate">
              {detail?.lastConnectedAt
                ? `Last: ${new Date(detail.lastConnectedAt).toLocaleString()}`
                : 'Never connected'}
            </span>
          </div>

          <Button size="sm" onClick={() => onConnect(connectionNode)}>
            <Play size={12} />
            Connect
          </Button>
        </div>
      </div>
    </div>
  );
}
