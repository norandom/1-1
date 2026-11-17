import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const ipc = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: ipc.invoke,
  Channel: class {
    onmessage: (payload: unknown) => void;
    constructor(listener: (payload: unknown) => void) { this.onmessage = listener; }
  },
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name, callback) => {
    ipc.listeners.set(name, callback);
    return () => ipc.listeners.delete(name);
  }),
}));
import { connectSession } from '../session-runtime';
import { getSessionInfo, listSessions } from '../tauri';
import { useSessionStore } from '../../stores/sessionStore';
import { statusesFromSessions, connectionStatusText } from '../tree-a11y';
import { buildSessionTabs } from '../document-tabs';
import { buildStatusBarText, statusBarIndicator, liveSessionCount } from '../status-bar';
import { SessionCanvas } from '../../components/session/SessionCanvas';

const reason = 'TCP connection failed: <remote> refused';
// Exact Rust SessionInfo/SessionState::Error serialization (externally tagged enum).
const wire = {
  id: 'session-failed', connectionId: 'failed', connectionName: 'Failed host', host: 'invalid.invalid',
  status: { error: reason }, lastError: reason, connectedAt: null,
  width: 800, height: 600, fps: 0, latency: 0, bandwidth: 0,
  reconnectAttempts: 0, maxReconnectAttempts: 5, displayStalled: false,
};
beforeEach(() => {
  vi.useFakeTimers();
  ipc.invoke.mockReset();
  ipc.listeners.clear();
  ipc.invoke.mockImplementation(async (command) => command === 'connect' ? wire.id : command === 'list_sessions' ? [wire] : wire);
});
afterEach(() => {
  for (const id of useSessionStore.getState().sessions.keys()) useSessionStore.getState().removeSession(id);
  vi.useRealTimers();
});

describe('Rust session error wire integration', () => {
  it.each(['snapshot', 'event'] as const)('normalizes %s through runtime/store into visible error state', async (source) => {
    if (source === 'event') ipc.invoke.mockImplementation(async (command) => command === 'connect' ? wire.id : null);
    await connectSession('failed');
    if (source === 'event') ipc.listeners.get(`session-info-${wire.id}`)!({ payload: wire });
    const sessions = useSessionStore.getState().sessions;
    const session = sessions.get(wire.id)!;
    expect(session.status).toBe('error');
    expect(session.lastError).toBe(reason);
    expect(connectionStatusText(statusesFromSessions(sessions.values()).failed)).toBe('Error');
    expect(buildSessionTabs(sessions.values(), {})[0].statusText).toBe('Error');
    expect(buildStatusBarText(session)).toBe('Connection error - failed');
    expect(statusBarIndicator(session)).toEqual({ label: 'Error', color: 'var(--color-danger)', pulse: false });
    expect(liveSessionCount(sessions.values())).toBe(0);
    const noop = () => {};
    const html = renderToStaticMarkup(createElement(SessionCanvas, {
      status: session.status, lastError: session.lastError, surfaceWidth: session.width, surfaceHeight: session.height,
      subscribeToFrames: () => noop, onKeyDown: noop, onKeyUp: noop, onMouseMove: noop,
      onMouseDown: noop, onMouseUp: noop, onWheel: noop,
    }));
    expect(html).toContain('Connection Error');
    expect(html).not.toContain('<canvas');
    expect(html).toContain('TCP connection failed: &lt;remote&gt; refused');
  });
  it('normalizes list snapshots through the same ingress contract', async () => {
    expect((await listSessions())[0]).toMatchObject({ status: 'error', lastError: reason });
  });
  it.each(['disconnected', 'connecting', 'connected', 'reconnecting'] as const)('preserves the %s wire state and metrics', async (status) => {
    ipc.invoke.mockResolvedValue({ ...wire, status, lastError: null, fps: 12, latency: 23, bandwidth: 34 });
    const info = await getSessionInfo(wire.id);
    expect(info).toMatchObject({ status, lastError: null, fps: 12, latency: 23, bandwidth: 34 });
    expect(statusBarIndicator(info).pulse).toBe(status === 'connected');
    expect(liveSessionCount(info ? [info] : [])).toBe(status === 'disconnected' ? 0 : 1);
  });
  it('uses the enum reason if lastError is absent, but preserves an existing cause', async () => {
    ipc.invoke.mockResolvedValueOnce({ ...wire, lastError: null });
    expect((await getSessionInfo(wire.id))?.lastError).toBe(reason);
    ipc.invoke.mockResolvedValueOnce({ ...wire, lastError: 'Detailed stored cause' });
    expect((await getSessionInfo(wire.id))?.lastError).toBe('Detailed stored cause');
  });
  it('preserves a missing session snapshot', async () => {
    ipc.invoke.mockResolvedValueOnce(null);
    expect(await getSessionInfo('missing')).toBeNull();
  });

});
