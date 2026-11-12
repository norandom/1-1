import { describe, it, expect, beforeEach } from 'vitest';

import { useSessionStore } from '../sessionStore';
import type { SessionInfo } from '../../types';

const session = (id: string, overrides: Partial<SessionInfo> = {}): SessionInfo => ({
  id,
  connectionId: `conn-${id}`,
  connectionName: `Conn ${id}`,
  host: 'host.invalid',
  status: 'connected',
  fps: 0,
  latency: 15,
  bandwidth: 0,
  width: 64,
  height: 64,
  reconnectAttempts: 0,
  maxReconnectAttempts: 5,
  lastError: null,
  ...overrides,
});

describe('sessionStore', () => {
  beforeEach(() => {
    useSessionStore.setState({
      sessions: new Map(),
      activeSessionId: null,
      showPerformanceHud: false,
      showQuickConnect: false,
    });
  });

  it('performance HUD is hidden by default and toggles (Req 4.3 gate)', () => {
    expect(useSessionStore.getState().showPerformanceHud).toBe(false);

    useSessionStore.getState().togglePerformanceHud();
    expect(useSessionStore.getState().showPerformanceHud).toBe(true);

    useSessionStore.getState().togglePerformanceHud();
    expect(useSessionStore.getState().showPerformanceHud).toBe(false);
  });

  it('quick-connect flag toggles and sets explicitly', () => {
    useSessionStore.getState().toggleQuickConnect();
    expect(useSessionStore.getState().showQuickConnect).toBe(true);

    useSessionStore.getState().setShowQuickConnect(false);
    expect(useSessionStore.getState().showQuickConnect).toBe(false);
  });

  it('addSession makes the new session active', () => {
    useSessionStore.getState().addSession(session('a'));
    useSessionStore.getState().addSession(session('b'));

    expect(useSessionStore.getState().activeSessionId).toBe('b');
  });

  it('removing the active session falls back to the oldest remaining, then null when empty', () => {
    const { addSession, removeSession } = useSessionStore.getState();
    addSession(session('a'));
    addSession(session('b'));
    addSession(session('c'));

    // Active session 'c' removed: the fallback is the first key in
    // insertion order ('a'), not the previously active session.
    removeSession('c');
    expect(useSessionStore.getState().activeSessionId).toBe('a');

    // Removing an inactive session keeps the active one.
    removeSession('b');
    expect(useSessionStore.getState().activeSessionId).toBe('a');

    removeSession('a');
    expect(useSessionStore.getState().activeSessionId).toBeNull();
    expect(useSessionStore.getState().sessions.size).toBe(0);
  });

  it('updateSession merges partial info and ignores unknown ids', () => {
    const { addSession, updateSession } = useSessionStore.getState();
    addSession(session('a'));

    updateSession('a', { fps: 42, status: 'reconnecting' });
    const updated = useSessionStore.getState().sessions.get('a');
    expect(updated?.fps).toBe(42);
    expect(updated?.status).toBe('reconnecting');
    expect(updated?.latency).toBe(15);

    updateSession('nope', { fps: 1 });
    expect(useSessionStore.getState().sessions.has('nope')).toBe(false);
  });

  it('setActive switches the active session', () => {
    const { addSession, setActive } = useSessionStore.getState();
    addSession(session('a'));
    addSession(session('b'));

    setActive('a');
    expect(useSessionStore.getState().activeSessionId).toBe('a');
  });
});
