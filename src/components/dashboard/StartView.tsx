import { useMemo } from 'react';
import { motion } from 'framer-motion';
import { Zap, Library, ArrowRight } from 'lucide-react';
import { startViewEmptyState } from '../../lib/dashboard-view';
import { statusesFromSessions } from '../../lib/tree-a11y';
import { buildRailEntries } from '../../lib/tree-rail';
import { useLibraryStore } from '../../stores/libraryStore';
import { useSessionStore } from '../../stores/sessionStore';
import { Button } from '@/components/ui/button';

/**
 * Task 8.5 (Req 11.5, design.md L397): the start view at `/` when no
 * tree node is selected — quick connect (the existing modal) plus one
 * shortcut per library that selects it (the selection then drives the
 * main panel to that library's dashboard).
 */
export function StartView() {
  const tree = useLibraryStore((s) => s.tree);
  const selectNode = useLibraryStore((s) => s.selectNode);
  const sessions = useSessionStore((s) => s.sessions);
  const setShowQuickConnect = useSessionStore((s) => s.setShowQuickConnect);

  const statuses = useMemo(() => statusesFromSessions(sessions.values()), [sessions]);
  const shortcuts = useMemo(() => buildRailEntries(tree, statuses), [tree, statuses]);
  const emptyState = startViewEmptyState(tree);

  return (
    <div className="flex h-full flex-col items-center justify-center px-6">
      <div className="w-full max-w-xl text-center">
        <h1 className="text-2xl font-bold tracking-tight text-foreground">1-1</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Connect to a machine to start a session
        </p>

        <div className="mt-6 flex justify-center">
          <Button size="lg" onClick={() => setShowQuickConnect(true)}>
            <Zap className="size-4" />
            Quick Connect
          </Button>
        </div>

        {emptyState ? (
          <p className="mt-10 text-sm text-muted-foreground">No libraries yet</p>
        ) : (
          <>
            <p className="mt-10 text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Libraries
            </p>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              {shortcuts.map((shortcut) => (
                <motion.button
                  key={shortcut.library.id}
                  type="button"
                  aria-label={shortcut.accessibleName}
                  whileHover={{ y: -2 }}
                  onClick={() => selectNode(shortcut.library)}
                  className="card-shadow flex items-center justify-between rounded-xl bg-card p-4 text-left ring-1 ring-foreground/10 transition-all hover:ring-foreground/20"
                >
                  <span className="flex min-w-0 items-center gap-3">
                    <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted">
                      <Library className="size-4 text-muted-foreground" />
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-medium text-foreground">
                        {shortcut.name}
                      </span>
                      {/* Textual aggregate — never color-only (10.4) */}
                      <span className="block text-[11px] text-muted-foreground">
                        {shortcut.statusText}
                      </span>
                    </span>
                  </span>
                  <ArrowRight className="size-4 shrink-0 text-muted-foreground" />
                </motion.button>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
