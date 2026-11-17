import { Sun, Moon, MonitorCog } from 'lucide-react';
import { useReducer } from 'react';
import { useSessionStore } from '../../stores/sessionStore';
import { useConnectionStore } from '../../stores/connectionStore';
import { useSettingsStore } from '../../stores/settingsStore';
import { useLocation } from 'react-router-dom';
import { useTheme } from '../../hooks/useTheme';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { buildStatusBarText, sessionCountLabel, statusBarIndicator, liveSessionCount, shouldAutoHideStatusBar, statusBarRevealReducer, statusBarRowHeight } from '../../lib/status-bar';

export function StatusBar() {
  const sessions = useSessionStore((s) => s.sessions);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const connections = useConnectionStore((s) => s.connections);
  const autoHide = useSettingsStore((s) => s.settings.autoHideStatusBar);
  const { pathname } = useLocation();
  const { theme, toggle } = useTheme();

  const activeSession = activeSessionId ? sessions.get(activeSessionId) : null;
  const sessionCount = liveSessionCount(sessions.values());
  const indicator = statusBarIndicator(activeSession ?? null);

  const connection = connections.find((c) => c.id === activeSession?.connectionId);
  const statusText = buildStatusBarText(activeSession ?? null, connection);
  const autoHideInSession = shouldAutoHideStatusBar(autoHide, pathname, sessions.values(), connections);
  const [reveal, dispatchReveal] = useReducer(statusBarRevealReducer, { pointer: false, focus: false });
  const rowHeight = statusBarRowHeight(autoHideInSession, reveal);

  return (
    <div id={autoHideInSession ? 'status-bar-focus-target' : undefined} role="region" aria-label="Connection status bar" aria-keyshortcuts={autoHideInSession ? 'Control+Alt+Shift+S' : undefined} tabIndex={autoHideInSession ? -1 : undefined}
      data-local-input data-status-bar-mode={autoHideInSession ? 'auto-hide' : 'visible'} className="shrink-0 focus-visible:outline-2 focus-visible:outline-primary focus-visible:outline-offset-[-2px]" style={{ height: rowHeight }}
      onPointerEnter={() => dispatchReveal('pointer-enter')} onPointerLeave={() => dispatchReveal('pointer-leave')}
      onFocusCapture={() => dispatchReveal('focus-enter')}
      onBlurCapture={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) dispatchReveal('focus-leave'); }}>
      {autoHideInSession && <button type="button" aria-label="Show status bar" title="Show status bar (Ctrl+Alt+Shift+S)" className="fixed inset-x-0 bottom-0 z-40 h-1 w-full opacity-0 focus-visible:opacity-100 focus-visible:bg-primary focus-visible:outline-none" />}
    <div
      className={`${rowHeight ? 'flex' : 'hidden'} items-center justify-between gap-2 px-3 border-t border-[var(--color-border)] bg-[var(--color-surface-1)] text-[10px] select-none`}
      style={{ height: 28 }}
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

      {/* Right: theme toggle */}
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
