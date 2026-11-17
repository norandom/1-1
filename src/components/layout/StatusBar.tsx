import { Sun, Moon, MonitorCog } from 'lucide-react';
import { useSessionStore } from '../../stores/sessionStore';
import { useConnectionStore } from '../../stores/connectionStore';
import { useSettingsStore } from '../../stores/settingsStore';
import { useLocation } from 'react-router-dom';
import { useTheme } from '../../hooks/useTheme';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { buildStatusBarText, sessionCountLabel, statusBarIndicator, liveSessionCount, shouldAutoHideStatusBar } from '../../lib/status-bar';

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
  const hidden = shouldAutoHideStatusBar(autoHide, pathname, sessions.values(), connections);

  return (
    <div data-local-input data-status-bar-mode={hidden ? 'auto-hide' : 'visible'} className={hidden ? 'group/status fixed inset-x-0 bottom-0 z-40 h-1' : 'shrink-0'}>
      {hidden && <button type="button" aria-label="Show status bar" className="absolute inset-0 w-full opacity-0 focus-visible:z-10 focus-visible:opacity-100 focus-visible:bg-primary focus-visible:outline-none" />}
    <div
      className={`flex items-center justify-between gap-2 px-3 border-t border-[var(--color-border)] bg-[var(--color-surface-1)] text-[10px] select-none ${hidden ? 'invisible absolute inset-x-0 bottom-0 pointer-events-none group-hover/status:visible group-hover/status:pointer-events-auto group-focus-within/status:visible group-focus-within/status:pointer-events-auto' : ''}`}
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
