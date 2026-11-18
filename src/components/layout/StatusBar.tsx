import { Sun, Moon, MonitorCog } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useSessionStore } from '../../stores/sessionStore';
import { useConnectionStore } from '../../stores/connectionStore';
import { sessionChrome, useSessionChrome } from '../../stores/sessionChromeStore';
import { useLocation } from 'react-router-dom';
import { useTheme } from '../../hooks/useTheme';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { cn } from '@/lib/utils';
import { ClipboardStatus } from './ClipboardStatus';
import { DisplayPauseControl } from './DisplayPauseControl';
import { sessionRouteIdFromPath } from '../../lib/session-route';
import { bottomBarFollowsToolbar, buildStatusBarText, sessionCountLabel, statusBarIndicator, liveSessionCount } from '../../lib/status-bar';

/**
 * The bottom control bar. By default it is a status row below the desktop.
 * A saved host can instead show it as an overlay line that appears and hides
 * with the top session toolbar, leaving the whole window to the desktop.
 */
export function StatusBar() {
  const sessions = useSessionStore((s) => s.sessions);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const connections = useConnectionStore((s) => s.connections);
  const chromeVisible = useSessionChrome((s) => s.visible);
  const { pathname } = useLocation();
  const { theme, toggle } = useTheme();

  const routeId = sessionRouteIdFromPath(pathname);
  const selectedSession = routeId ? sessions.get(routeId) : undefined;
  const activeSession = selectedSession ?? (activeSessionId ? sessions.get(activeSessionId) : null);
  const sessionCount = liveSessionCount(sessions.values());
  const indicator = statusBarIndicator(activeSession ?? null);

  const connection = connections.find((c) => c.id === activeSession?.connectionId);
  const statusText = buildStatusBarText(activeSession ?? null, connection);
  const overlay = bottomBarFollowsToolbar(pathname, sessions.values(), connections);
  const [focused, setFocused] = useState(false);
  // Holds only apply to the overlay; drop them when its mode or view changes.
  useEffect(() => () => {
    setFocused(false);
    for (const reason of ['bar-hover', 'bar-focus', 'bar-menu'] as const) sessionChrome.release(reason);
  }, [overlay, pathname]);
  const shown = !overlay || chromeVisible || focused;
  const hasSessionShortcut = overlay || !!selectedSession;
  const hold = (reason: 'bar-hover' | 'bar-focus' | 'bar-menu', on: boolean) => {
    if (!overlay) return;
    if (on) sessionChrome.hold(reason);
    else sessionChrome.release(reason);
  };

  return (
    <div id={hasSessionShortcut ? 'status-bar-focus-target' : undefined} role="region" aria-label="Connection status bar" aria-keyshortcuts={hasSessionShortcut ? 'Control+Alt+Shift+S' : undefined} tabIndex={hasSessionShortcut ? -1 : undefined}
      data-local-input data-status-bar-mode={overlay ? 'with-toolbar' : 'visible'}
      className={cn(
        'focus-visible:outline-2 focus-visible:outline-primary focus-visible:outline-offset-[-2px]',
        overlay
          ? cn('fixed inset-x-0 bottom-0 z-30 transition-[transform,opacity] duration-200 ease-out', shown ? 'translate-y-0 opacity-100' : 'pointer-events-none translate-y-full opacity-0')
          : 'shrink-0',
      )}
      style={overlay ? undefined : { height: 28 }}
      onPointerEnter={() => hold('bar-hover', true)} onPointerLeave={() => hold('bar-hover', false)}
      onFocusCapture={() => { setFocused(true); hold('bar-focus', true); }}
      onBlurCapture={(event) => {
        if (event.currentTarget.contains(event.relatedTarget)) return;
        setFocused(false);
        hold('bar-focus', false);
      }}>
    <div
      className={cn(
        'flex items-center justify-between gap-2 px-3 border-t border-[var(--color-border)] text-[10px] select-none',
        overlay ? 'glass shadow-2xl' : 'bg-[var(--color-surface-1)]',
      )}
      style={{ height: 28 }}
      aria-hidden={shown ? undefined : true}
    >
      {/* Left: connection status */}
      <div className="flex min-w-0 items-center gap-2 text-[var(--color-text-muted)]">
        <Badge
          variant="outline"
          className="h-4 shrink-0 gap-1 px-1.5 text-[10px]"
          style={{ color: indicator.color, borderColor: indicator.color }}
        >
          <span
            className={`inline-block w-1.5 h-1.5 rounded-full${indicator.pulse ? ' pulse-online' : ''}`}
            style={{ backgroundColor: indicator.color }}
          />
          {indicator.label}
        </Badge>
        <span className="truncate" title={statusText}>{statusText}</span>
      </div>

      <Separator orientation="vertical" className="h-3.5" />

      {/* Center: session count */}
      <div className="flex shrink-0 items-center gap-1.5 whitespace-nowrap text-[var(--color-text-muted)]">
        <MonitorCog size={11} />
        <span>
          {sessionCountLabel(sessionCount)}
        </span>
      </div>

      <Separator orientation="vertical" className="h-3.5" />

      <DisplayPauseControl session={selectedSession} />
      <ClipboardStatus onOpenChange={(open) => hold('bar-menu', open)} />
      <Button
        variant="ghost"
        size="xs"
        className="h-5 shrink-0 gap-1.5 text-[10px] text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]"
        onClick={toggle}
        title={`Theme: ${theme}`}
      >
        {theme === 'dark' ? <Moon size={11} /> : theme === 'light' ? <Sun size={11} /> : <MonitorCog size={11} />}
        <span className="capitalize">{theme}</span>
      </Button>
    </div>
    </div>
  );
}
