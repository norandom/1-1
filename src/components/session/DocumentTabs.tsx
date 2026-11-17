import { useEffect, useMemo, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import { useLocation, useNavigate } from 'react-router-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { Monitor, Plus, X } from 'lucide-react';
import { useConnectionStore } from '../../stores/connectionStore';
import { useLibraryStore } from '../../stores/libraryStore';
import { useSessionStore } from '../../stores/sessionStore';
import * as tauri from '../../lib/tauri';
import {
  activeTabIdFromPath,
  buildSessionTabs,
  closeFocusRoute,
  nextTabFocus,
  resolveCloseFocus,
  tabAccessibleName,
  type TabFocusKey,
} from '../../lib/document-tabs';
import { sessionRoute } from '../../lib/session-route';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

const FOCUS_KEYS: readonly TabFocusKey[] = ['ArrowLeft', 'ArrowRight', 'Home', 'End'];

/**
 * Task 8.6 (Req 11.1-11.3, 11.7, design.md L396): the persistent
 * document tab strip, hoisted above the routes so it stays visible
 * across navigation. One tab per `sessionStore.sessions` entry — the
 * session id IS the tab identity (11.4). All derivation (tabs, active
 * state from the route, close-focus resolution, keyboard moves) lives
 * in the node-tested `src/lib/document-tabs.ts`; this render layer
 * stays thin against TAB_LIST_STATE_PINS / CLOSE_TAB_PINS.
 */
export function DocumentTabs() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const sessions = useSessionStore((s) => s.sessions);
  const setActive = useSessionStore((s) => s.setActive);
  const removeSession = useSessionStore((s) => s.removeSession);
  const clearRestartRequest = useSessionStore((s) => s.clearRestartRequest);
  const connections = useConnectionStore((s) => s.connections);
  const selectedNodeId = useLibraryStore((s) => s.selectedNodeId);

  const [keyboardFocusTabId, setKeyboardFocusTabId] = useState<string | null>(null);
  // Remediation finding 6: tabs are div[role="tab"] elements, not
  // <button>s — a <button> cannot contain the close <button> (invalid
  // nested interactive content, broken 11.7 a11y tree). All keyboard
  // activation is handled explicitly below.
  const tabRefs = useRef(new Map<string, HTMLDivElement>());
  const pendingFocusRef = useRef<string | null>(null);

  const connectionNames = useMemo(() => {
    const names: Record<string, string> = {};
    for (const c of connections) names[c.id] = c.name;
    return names;
  }, [connections]);

  const tabs = useMemo(
    () => buildSessionTabs(sessions.values(), connectionNames),
    [sessions, connectionNames]
  );
  const tabIds = useMemo(() => tabs.map((t) => t.tabId), [tabs]);

  // No open sessions: no strip (the start view owns the panel, 11.5).
  const hasTabs = tabs.length > 0;

  // The active tab is the session the route points at (11.2); tabs
  // stay open — just not active — over dashboards (11.6).
  const activeTabId = activeTabIdFromPath(pathname, tabIds);
  const focusedTabId =
    keyboardFocusTabId && tabIds.includes(keyboardFocusTabId) ? keyboardFocusTabId : null;
  const rovingTabId = focusedTabId ?? activeTabId ?? tabIds[0] ?? null;

  // After a close, focus must land on the resolved adjacent tab.
  useEffect(() => {
    const pending = pendingFocusRef.current;
    if (!pending || !tabIds.includes(pending)) return;
    pendingFocusRef.current = null;
    setKeyboardFocusTabId(pending);
    tabRefs.current.get(pending)?.focus();
  }, [tabIds]);

  const handleTabClick = (sessionId: string) => {
    // Remediation finding 5 (Req 11.9): selecting a tab is NOT a
    // Connect gesture — consume any pending restart request so the
    // selection can never auto-restart a terminal session.
    clearRestartRequest();
    setActive(sessionId);
    navigate(sessionRoute(sessionId));
  };

  const handleTabKeyDown = (event: React.KeyboardEvent, tabId: string) => {
    // Let the nested close button handle its own Enter/Space activation.
    if (event.target !== event.currentTarget) return;
    // The tab is a div[role="tab"]: activate it on Enter/Space exactly
    // like the native button it replaced.
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      handleTabClick(tabId);
      return;
    }
    if (!FOCUS_KEYS.includes(event.key as TabFocusKey)) return;
    event.preventDefault();
    const target = nextTabFocus(tabIds, focusedTabId ?? tabId, event.key as TabFocusKey);
    if (!target) return;
    setKeyboardFocusTabId(target);
    tabRefs.current.get(target)?.focus();
  };

  const handleClose = async (sessionId: string) => {
    // Req 11.3/11.7 (CLOSE_TAB_PINS): disconnect exactly this session,
    // remove exactly its tab, then focus the adjacent tab — next first,
    // else previous — or the dashboard/start view.
    const focus = resolveCloseFocus(tabIds, sessionId, selectedNodeId);
    try {
      await tauri.disconnect(sessionId);
    } catch {
      // The backend session may already be gone; the tab still closes.
    }
    if (focus.focus === 'adjacent-tab') {
      pendingFocusRef.current = focus.tabId;
    }
    // Commit route and registry changes together. Removing the active session
    // first exposes a stale route and lets SessionView redirect to `/`, racing
    // the intended adjacent-tab navigation.
    flushSync(() => {
      navigate(closeFocusRoute(focus));
      removeSession(sessionId);
    });
    if (focus.focus !== 'adjacent-tab') {
      // Dashboard / start view: hand focus to the main panel.
      requestAnimationFrame(() => document.getElementById('main-panel')?.focus());
    }
  };

  if (!hasTabs) return null;

  return (
    <div
      role="tablist"
      aria-label="Open sessions"
      className="flex h-9 shrink-0 items-center overflow-x-auto border-b border-[var(--color-border)] bg-[var(--color-surface-1)]"
    >
      <AnimatePresence mode="popLayout" initial={false}>
        {tabs.map((tab) => {
          const isActive = tab.tabId === activeTabId;
          return (
            <motion.div
              key={tab.tabId}
              layout
              initial={{ opacity: 0, width: 0 }}
              animate={{ opacity: 1, width: 'auto' }}
              exit={{ opacity: 0, width: 0 }}
              transition={{ duration: 0.2 }}
              role="tab"
              aria-selected={isActive}
              aria-label={tabAccessibleName(tab)}
              tabIndex={tab.tabId === rovingTabId ? 0 : -1}
              ref={(el) => {
                if (el) tabRefs.current.set(tab.tabId, el);
                else tabRefs.current.delete(tab.tabId);
              }}
              onClick={() => handleTabClick(tab.tabId)}
              onFocus={() => setKeyboardFocusTabId(tab.tabId)}
              onKeyDown={(e) => handleTabKeyDown(e, tab.tabId)}
              className={cn(
                'relative flex h-full cursor-pointer items-center gap-2 whitespace-nowrap border-r border-[var(--color-border)] px-4 text-xs font-medium outline-none transition-colors',
                'focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--color-accent)]',
                isActive
                  ? 'bg-[var(--color-surface-0)] text-[var(--color-text-primary)]'
                  : 'text-[var(--color-text-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text-secondary)]'
              )}
            >
              {isActive && (
                <motion.div
                  layoutId="active-tab-indicator"
                  className="absolute bottom-0 left-0 right-0 h-[2px] bg-[var(--color-accent)]"
                />
              )}
              <Monitor size={12} />
              <span>{tab.name}</span>
              {/* Textual live status — never color-only (10.4). */}
              <span className="text-[10px] font-normal text-[var(--color-text-muted)]">
                {tab.statusText}
              </span>
              {/* Remediation finding 6 (11.7): the named close control
                  is a real button inside the div[role="tab"] — valid
                  markup now; naming ("Close <name>") and the pinned
                  CLOSE_TAB_PINS focus outcomes are unchanged. */}
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={tab.closeControlName}
                title={tab.closeControlName}
                tabIndex={tab.tabId === rovingTabId ? 0 : -1}
                className="h-4 w-4 rounded p-0 hover:bg-[var(--color-surface-3)]"
                onClick={(e) => {
                  e.stopPropagation();
                  void handleClose(tab.tabId);
                }}
              >
                <X size={10} />
              </Button>
            </motion.div>
          );
        })}
      </AnimatePresence>

      {/* New tab button */}
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="New tab"
        className="h-full shrink-0 rounded-none text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]"
        onClick={() => navigate('/')}
      >
        <Plus size={14} />
      </Button>
    </div>
  );
}
