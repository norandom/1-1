import { Sun, Moon, MonitorCog } from 'lucide-react';
import { useSessionStore } from '../../stores/sessionStore';
import { useTheme } from '../../hooks/useTheme';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { buildStatusBarText, sessionCountLabel, statusBarIndicator, liveSessionCount } from '../../lib/status-bar';

export function StatusBar() {
  const sessions = useSessionStore((s) => s.sessions);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const { theme, toggle } = useTheme();

  const activeSession = activeSessionId ? sessions.get(activeSessionId) : null;
  const sessionCount = liveSessionCount(sessions.values());
  const indicator = statusBarIndicator(activeSession ?? null);

  const statusText = buildStatusBarText(activeSession ?? null);

  return (
    <div
      className="flex items-center justify-between px-3 shrink-0 border-t border-[var(--color-border)] bg-[var(--color-surface-1)] text-[10px] select-none"
      style={{ height: 28 }}
    >
      {/* Left: connection status */}
      <div className="flex items-center gap-2 text-[var(--color-text-muted)]">
        <Badge
          variant="outline"
          className="h-4 gap-1 px-1.5 text-[10px]"
          style={{ color: indicator.color, borderColor: indicator.color }}
        >
          <span
            className={`inline-block w-1.5 h-1.5 rounded-full${indicator.pulse ? ' pulse-online' : ''}`}
            style={{ backgroundColor: indicator.color }}
          />
          {indicator.label}
        </Badge>
        <span>{statusText}</span>
      </div>

      <Separator orientation="vertical" className="h-3.5" />

      {/* Center: session count */}
      <div className="flex items-center gap-1.5 text-[var(--color-text-muted)]">
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
        className="h-5 gap-1.5 text-[10px] text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]"
        onClick={toggle}
        title={`Theme: ${theme}`}
      >
        {theme === 'dark' ? <Moon size={11} /> : theme === 'light' ? <Sun size={11} /> : <MonitorCog size={11} />}
        <span className="capitalize">{theme}</span>
      </Button>
    </div>
  );
}
