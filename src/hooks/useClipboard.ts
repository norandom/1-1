import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import * as tauri from '../lib/tauri';
import { createClipboardController } from '../lib/clipboard-controller';
import { useClipboardStore } from '../stores/clipboardStore';
import { useSessionStore } from '../stores/sessionStore';
import { subscribeClipboardInput } from '../lib/clipboard-input';

export function useClipboard({ sessionId, status }: { sessionId: string | null; status: string }) {
  useEffect(() => {
    if (!sessionId || status !== 'connected') return;
    let disposed = false;
    const cleanup: (() => void)[] = [];
    const active = () => !disposed && useSessionStore.getState().activeSessionId === sessionId && useSessionStore.getState().sessions.get(sessionId)?.status === 'connected';
    const setup = async () => {
      try {
        const { direction, epoch } = await tauri.clipboardContext(sessionId);
        if (!active()) return;
        const controller = createClipboardController({ direction,
          allowIncoming: () => document.activeElement?.tagName === 'CANVAS',
          read: async () => {
            if (!active()) throw new Error('Session changed.');
            try { return await navigator.clipboard.readText(); }
            catch { throw new Error('Cannot read the local clipboard. Focus 1-1 and retry resync to remote.'); }
          },
          write: async (text) => { if (active()) await navigator.clipboard.writeText(text); },
          publish: async (text) => { if (active()) await tauri.clipboardSync(sessionId, epoch, text); },
          request: async () => { if (active()) await tauri.clipboardSync(sessionId, epoch, null); },
          paste: async () => { if (active()) await tauri.clipboardPaste(sessionId, epoch); },
          state: (next) => { if (active()) useClipboardStore.setState({ status: next }); },
        });
        useClipboardStore.setState({ sessionId, direction, controller, status: { kind: 'idle', message: direction === 'off' ? 'Clipboard off' : 'Clipboard ready' } });
        cleanup.push(() => {
          controller.dispose();
          if (useClipboardStore.getState().controller === controller) useClipboardStore.setState({ sessionId: null, controller: null, direction: 'off', status: { kind: 'idle', message: 'Clipboard unavailable' } });
        });
        const register = async (name: string, callback: (value: { text?: string; message?: string }) => void) => {
          const unlisten = await listen<{ epoch: string; text?: string; message?: string }>(name, (event) => { if (active() && event.payload.epoch === epoch) callback(event.payload); });
          if (disposed) unlisten(); else cleanup.push(unlisten);
        };
        await register(`clipboard-${sessionId}`, ({ text }) => { if (text !== undefined) void controller.receive(text); });
        await register(`clipboard-error-${sessionId}`, ({ message }) => controller.fail(message ?? 'Clipboard failed. Retry resync.'));
        if (!active()) return;
        cleanup.push(subscribeClipboardInput(window, document, direction, active, controller));
      } catch { if (active()) useClipboardStore.setState({ sessionId, status: { kind: 'error', message: 'Clipboard setup failed. Reconnect the session.' } }); }
    };
    void setup();
    return () => { disposed = true; cleanup.forEach((fn) => fn()); };
  }, [sessionId, status]);
}
