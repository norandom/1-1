import { useEffect } from 'react';
import { useSessionStore } from '../stores/sessionStore';
import { matchKeyboardShortcut } from '../lib/shortcuts';

interface KeyboardShortcuts {
  onNewConnection?: () => void;
  onShowShortcuts?: () => void;
}

export function useKeyboard({ onNewConnection, onShowShortcuts }: KeyboardShortcuts = {}) {
  const toggleQuickConnect = useSessionStore((s) => s.toggleQuickConnect);
  const togglePerformanceHud = useSessionStore((s) => s.togglePerformanceHud);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Ctrl and Cmd respond identically (Req 8.5): the matcher treats
      // metaKey and ctrlKey as the same platform modifier.
      switch (matchKeyboardShortcut(e)) {
        case 'quick-connect':
          e.preventDefault();
          toggleQuickConnect();
          return;
        case 'new-connection':
          e.preventDefault();
          onNewConnection?.();
          return;
        case 'performance-hud':
          e.preventDefault();
          togglePerformanceHud();
          return;
        case 'shortcut-help':
          e.preventDefault();
          onShowShortcuts?.();
          return;
        default:
          return;
      }
    }

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [toggleQuickConnect, togglePerformanceHud, onNewConnection, onShowShortcuts]);
}
