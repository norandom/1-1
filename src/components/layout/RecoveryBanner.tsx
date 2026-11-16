import { AlertTriangle, Loader2, RefreshCw } from 'lucide-react';
import { useLibraryStore } from '../../stores/libraryStore';
import { recoveryViewModel } from '../../lib/recovery';
import { Button } from '@/components/ui/button';

/**
 * Task 8.7 (Req 10.12): the persistent recovery instructions view. While
 * the library store is write-blocked (failed load/migration) this banner
 * stays visible across all navigation (mounted above the routes in App,
 * like the 8.6 tab strip) and cannot be dismissed — visibility derives
 * solely from the backend status. It presents the backend reason and
 * recovery instructions, the preserved-originals note, and the Reload
 * action; controls restore only after that reload succeeds. All logic
 * lives in the tested view-model (`lib/recovery.ts`); rendered behavior
 * is verified by the group-12 E2E tier per the design testing strategy.
 */
export function RecoveryBanner() {
  const status = useLibraryStore((s) => s.status);
  const isReloading = useLibraryStore((s) => s.isReloading);
  const recoveryError = useLibraryStore((s) => s.recoveryError);
  const reload = useLibraryStore((s) => s.reload);

  const model = recoveryViewModel(status);
  if (!model.visible) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      aria-label={model.title ?? undefined}
      className="shrink-0 border-b border-[var(--color-danger)]/40 bg-[var(--color-surface-1)] px-4 py-3"
    >
      <div className="flex items-start gap-3">
        <AlertTriangle
          size={16}
          className="mt-0.5 shrink-0 text-[var(--color-warning)]"
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1">
          <p className="text-[13px] font-medium text-foreground">{model.title}</p>
          {model.reason && (
            <p className="mt-1 text-[12px] text-[var(--color-text-secondary)]">{model.reason}</p>
          )}
          {model.instructions && (
            <p className="mt-1 text-[12px] text-foreground">{model.instructions}</p>
          )}
          {model.preservedNote && (
            <p className="mt-1 text-[12px] text-[var(--color-text-muted)]">{model.preservedNote}</p>
          )}
          {recoveryError && (
            <p role="alert" className="mt-1 text-[12px] text-[var(--color-danger)]">
              Reload failed: {recoveryError}
            </p>
          )}
        </div>
        {/* The only path out of recovery (Req 10.12): re-run the load. */}
        <Button
          size="sm"
          variant="outline"
          className="shrink-0"
          disabled={isReloading}
          onClick={() => {
            void reload();
          }}
        >
          {isReloading ? (
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          ) : (
            <RefreshCw className="size-4" aria-hidden="true" />
          )}
          {model.reloadLabel}
        </Button>
      </div>
    </div>
  );
}
