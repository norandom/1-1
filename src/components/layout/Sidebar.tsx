import { useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { motion } from 'framer-motion';
import { Library as LibraryIcon } from 'lucide-react';
import { useSessionStore } from '../../stores/sessionStore';
import { useSettingsStore } from '../../stores/settingsStore';
import { useLibraryStore } from '../../stores/libraryStore';
import { statusesFromSessions } from '../../lib/tree-a11y';
import {
  buildRailEntries,
  nextSidebarCollapsed,
  railToggleButtonName,
} from '../../lib/tree-rail';
import { ConnectionTree } from './ConnectionTree';
import { NewLibraryDialog } from './NewLibraryDialog';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

/**
 * Task 8.3 (Req 10.1/10.11): the sidebar hosts the ConnectionTree; the
 * explicit named collapse control shrinks it to a rail of library icons
 * with status dots (textual status stays in every accessible name).
 * Collapsing never touches tree state — selection and saved expansion
 * live in libraryStore and the preference persists through
 * AppSettings.sidebarCollapsed (RAIL_STATE_PINS).
 */

const EXPANDED_WIDTH = 260;
const COLLAPSED_WIDTH = 72;

function SettingsIcon({ size = 20, className }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" className={className}>
      <path d="M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

function CollapseLeftIcon({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M9 3v18" />
      <path d="M16 15l-3-3 3-3" />
    </svg>
  );
}

function CollapseRightIcon({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M9 3v18" />
      <path d="M14 9l3 3-3 3" />
    </svg>
  );
}

export function Sidebar() {
  const location = useLocation();
  const navigate = useNavigate();
  const collapsed = useSettingsStore((s) => s.settings.sidebarCollapsed);
  const updateSettings = useSettingsStore((s) => s.updateSettings);

  const sessions = useSessionStore((s) => s.sessions);
  const tree = useLibraryStore((s) => s.tree);
  const selectNode = useLibraryStore((s) => s.selectNode);

  // Rail entries carry the textual aggregate in their accessible names.
  const statuses = useMemo(() => statusesFromSessions(sessions.values()), [sessions]);
  const railEntries = useMemo(() => buildRailEntries(tree, statuses), [tree, statuses]);
  const libraryHasConnected = (libraryId: string): boolean =>
    tree.connections.some((c) => c.libraryId === libraryId && statuses[c.id] === 'connected');

  const isSettingsActive = location.pathname.startsWith('/settings');
  const toggleName = railToggleButtonName(collapsed);

  const toggleCollapse = () => {
    updateSettings({ sidebarCollapsed: nextSidebarCollapsed(collapsed) });
  };

  return (
    <motion.aside
      initial={false}
      animate={{ width: collapsed ? COLLAPSED_WIDTH : EXPANDED_WIDTH }}
      transition={{ duration: 0.2, ease: [0.25, 0.1, 0.25, 1] }}
      className="flex flex-col shrink-0 border-r border-[var(--color-border)] bg-[var(--color-surface-1)] overflow-hidden"
      style={{ height: '100%' }}
    >
      {/* Brand */}
      <div className="flex items-center gap-3.5 px-5 py-6 shrink-0">
        <div className="flex items-center justify-center w-10 h-10 rounded-xl shrink-0 shadow-md overflow-hidden">
          <svg width="40" height="40" viewBox="0 0 32 32" fill="none">
            <rect width="32" height="32" rx="8" fill="url(#drift-sb)" />
            <path d="M9 16C9 12.134 12.134 9 16 9C19.866 9 23 12.134 23 16" stroke="white" strokeWidth="2" strokeLinecap="round" opacity="0.4" />
            <path d="M11 16C11 13.239 13.239 11 16 11C18.761 11 21 13.239 21 16" stroke="white" strokeWidth="2" strokeLinecap="round" opacity="0.7" />
            <circle cx="16" cy="16" r="2.5" fill="white" />
            <path d="M16 18.5V24" stroke="white" strokeWidth="2" strokeLinecap="round" />
            <path d="M13 22L16 24L19 22" stroke="white" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" opacity="0.6" />
            <defs>
              <linearGradient id="drift-sb" x1="0" y1="0" x2="32" y2="32">
                <stop offset="0%" stopColor="#6366f1" />
                <stop offset="50%" stopColor="#8b5cf6" />
                <stop offset="100%" stopColor="#a855f7" />
              </linearGradient>
            </defs>
          </svg>
        </div>
        {!collapsed && (
          <span className="text-[15px] font-bold text-[var(--color-text-primary)] whitespace-nowrap overflow-hidden tracking-tight">
            Drift
          </span>
        )}
      </div>

      <Separator className="mx-4 mb-3 w-auto" />
      <div className="flex flex-col px-4 pb-2">
        <NewLibraryDialog collapsed={collapsed} />
      </div>

      {/* Tree (expanded) or library rail (collapsed) — same tree state */}
      {collapsed ? (
        <nav className="flex min-h-0 flex-1 flex-col items-center gap-1 overflow-y-auto px-2" aria-label="Libraries">
          {railEntries.map((entry) => {
            const connected = libraryHasConnected(entry.library.id);
            const railButton = (
              <Button
                variant="ghost"
                aria-label={entry.accessibleName}
                className="relative h-11 w-11 rounded-xl"
                onClick={() => {
                  // Task 8.5 (Req 10.5/11.6): a rail library click selects
                  // it and shows its dashboard, sessions untouched.
                  selectNode(entry.library);
                  if (location.pathname !== '/') navigate('/');
                }}
              >
                <LibraryIcon size={20} />
                <span
                  aria-hidden="true"
                  className={cn(
                    'absolute top-1.5 right-1.5 h-2 w-2 rounded-full',
                    connected ? 'bg-[var(--color-success)]' : 'bg-[var(--color-text-muted)]'
                  )}
                />
              </Button>
            );
            return (
              <Tooltip key={entry.library.id}>
                <TooltipTrigger asChild>{railButton}</TooltipTrigger>
                <TooltipContent side="right" sideOffset={8}>
                  {entry.accessibleName}
                </TooltipContent>
              </Tooltip>
            );
          })}
          <Separator className="my-1 w-8" />
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                aria-label="Settings"
                className="h-11 w-11 rounded-xl"
                onClick={() => navigate('/settings')}
              >
                <SettingsIcon size={20} />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              Settings
            </TooltipContent>
          </Tooltip>
        </nav>
      ) : (
        <div className="min-h-0 flex-1">
          <ConnectionTree />
        </div>
      )}

      {/* Settings (expanded) + named collapse control (Req 10.11) */}
      <div className="flex flex-col gap-1 border-t border-[var(--color-border)] px-4 py-3">
        {!collapsed && (
          <Button
            variant="ghost"
            aria-current={isSettingsActive ? 'page' : undefined}
            className="h-auto w-full justify-start gap-3 rounded-xl px-3.5 py-2.5"
            onClick={() => navigate('/settings')}
          >
            <SettingsIcon size={18} />
            <span className="text-[13px] font-semibold">Settings</span>
          </Button>
        )}
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              aria-label={toggleName}
              className="w-full justify-center rounded-xl py-2.5"
              onClick={toggleCollapse}
            >
              {collapsed ? <CollapseRightIcon /> : <CollapseLeftIcon />}
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right" sideOffset={8}>
            {toggleName}
          </TooltipContent>
        </Tooltip>
      </div>
    </motion.aside>
  );
}
