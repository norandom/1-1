import type { ClipboardPayload } from '../types';
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
        const lease = await tauri.clipboardLocalAcquire(sessionId, epoch);
        if (!active()) { await tauri.clipboardLocalRelease(lease); return; }
        cleanup.push(() => { void tauri.clipboardLocalRelease(lease).catch(() => {}); });
        const controller = createClipboardController({ direction,
          allowIncoming: () => document.activeElement?.tagName === 'CANVAS',
          read: async () => {
            if (!active()) throw new Error('Session changed.');
            try { return await tauri.clipboardLocalRead(lease); }
            catch (error) { throw new Error(typeof error === 'string' ? error : 'Cannot read native clipboard text or images. Copy again and retry resync to remote.'); }
          },
          write: async (text) => { if (active()) await tauri.clipboardLocalWrite(lease, text); },
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
        const register = async (name: string, callback: (value: { payload?: ClipboardPayload; message?: string }) => void) => {
          const unlisten = await listen<{ epoch: string; payload?: ClipboardPayload; message?: string }>(name, (event) => { if (active() && event.payload.epoch === epoch) callback(event.payload); });
          if (disposed) unlisten(); else cleanup.push(unlisten);
        };
        await register(`clipboard-${sessionId}`, ({ payload }) => { if (payload !== undefined) void controller.receive(payload); });
        await register(`clipboard-error-${sessionId}`, ({ message }) => controller.fail(message ?? 'Clipboard failed. Retry resync.'));
        if (!active()) return;
        cleanup.push(subscribeClipboardInput(window, document, direction, active, controller));
      } catch { if (active()) useClipboardStore.setState({ sessionId, status: { kind: 'error', message: 'Clipboard setup failed. Reconnect the session.' } }); }
    };
    void setup();
    return () => { disposed = true; cleanup.forEach((fn) => fn()); };
  }, [sessionId, status]);
}
