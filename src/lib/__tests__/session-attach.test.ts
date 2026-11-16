import { describe, expect, it, vi } from 'vitest';

import type { SessionInfo, SessionStatus } from '../../types';
import { resolveSessionView, sessionForConnection } from '../session-route';
import {
  createConnectController,
  resolveConnectAction,
  resolveUrlAction,
} from '../session-attach';

// Task 8.6 remediation round 1 (reviewer findings 1-3): the PURE
// SessionView decisions — when the transitional `/session/:connectionId`
// URL must be swapped for the session route the backend returned, and
// when the connect effect may fire IPC at all. SessionView stays
// render-thin and executes exactly these decisions; the tests pin the
// swap decision (finding 1) and the reconnect loop guard (finding 2).
//
// Task 8.6 remediation round 2 (reviewer findings 3+4): the once-only
// connect guard had no failure reset — a rejected connect left the
// route silently dead (no re-evaluation, guard masking the decision)
// and a route queued behind an in-flight connect never fired after the
// predecessor failed. The tests below pin the failure-recovery
// semantics: a NEW Connect gesture re-fires the connect (never an
// auto-retry), and a queued route connects once the predecessor
// settles without a session.

function session(id: string, connectionId: string, status: SessionStatus): SessionInfo {
  return {
    id,
    connectionId,
    connectionName: '',
    host: '',
    status,
    fps: 0,
    latency: 0,
    bandwidth: 0,
    width: 0,
    height: 0,
    reconnectAttempts: 0,
    maxReconnectAttempts: 5,
    lastError: null,
  };
}

const isKnownConnection = () => true;

describe('resolveUrlAction — the transitional-URL swap (finding 1, Req 11.4)', () => {
  it('a completed fresh connect replace-navigates the transitional connection-id route to the session route', () => {
    // Route /session/conn-bastion (kind 'connect'); the connect completed
    // and both the store and the hook hold the backend's session id.
    const resolution = resolveSessionView('conn-bastion', [], isKnownConnection);
    const existing = sessionForConnection(
      [session('session-bastion', 'conn-bastion', 'connected')],
      'conn-bastion'
    );
    expect(resolveUrlAction(resolution, existing, 'session-bastion')).toEqual({
      action: 'replace',
      route: '/session/session-bastion',
    });
  });

  it('a view resolution never swaps — the URL already names the session (11.4)', () => {
    const s = session('session-bastion', 'conn-bastion', 'connected');
    const resolution = resolveSessionView('session-bastion', [s], isKnownConnection);
    expect(resolveUrlAction(resolution, undefined, 'session-bastion')).toEqual({
      action: 'none',
    });
    expect(resolveUrlAction(resolution, s, 'session-bastion')).toEqual({ action: 'none' });
    expect(resolveUrlAction(resolution, s, 'session-jump')).toEqual({ action: 'none' });
  });

  it('a stale resolution redirects to the dashboard (11.5)', () => {
    const resolution = resolveSessionView('session-gone', [], () => false);
    expect(resolveUrlAction(resolution, undefined, null)).toEqual({
      action: 'replace',
      route: '/',
    });
  });

  it('a connect route swaps only once the hook holds the connection’s session', () => {
    const resolution = resolveSessionView('conn-bastion', [], isKnownConnection);
    const existing = session('session-bastion', 'conn-bastion', 'connecting');
    // No session for the connection yet:
    expect(resolveUrlAction(resolution, undefined, null)).toEqual({ action: 'none' });
    // Store has it but this hook does not hold it (swap still pending):
    expect(resolveUrlAction(resolution, existing, null)).toEqual({ action: 'none' });
    expect(resolveUrlAction(resolution, existing, 'session-other')).toEqual({ action: 'none' });
  });
});

describe('resolveConnectAction — the reconnect loop guard (finding 2)', () => {
  it('a fresh transitional route with no session initiates the connect', () => {
    const resolution = resolveSessionView('conn-bastion', [], isKnownConnection);
    expect(resolveConnectAction(resolution, undefined, null, null)).toEqual({
      action: 'connect',
      connectionId: 'conn-bastion',
    });
  });

  it('post-connect store updates never re-fire the connect (the guard)', () => {
    // The connect completed; the route is still transitional (the swap
    // is pending). Every sessionStore write — the session-info listener
    // and the 1 Hz fps interval — swaps the Map identity while the
    // content keeps naming the same session; the decision must hold on
    // content, never re-issue connect IPC.
    const resolution = resolveSessionView('conn-bastion', [], isKnownConnection);
    const churnedStores = [
      new Map([['session-bastion', session('session-bastion', 'conn-bastion', 'connected')]]),
      new Map([
        ['session-bastion', { ...session('session-bastion', 'conn-bastion', 'connected'), fps: 12 }],
      ]),
      new Map([
        [
          'session-bastion',
          { ...session('session-bastion', 'conn-bastion', 'connected'), fps: 60, latency: 20 },
        ],
      ]),
    ];
    for (const store of churnedStores) {
      const existing = sessionForConnection(store.values(), 'conn-bastion');
      expect(resolveConnectAction(resolution, existing, 'session-bastion', null)).toEqual({
        action: 'hold',
      });
    }
  });

  it('a connect already initiated for this transitional route is not repeated', () => {
    const resolution = resolveSessionView('conn-bastion', [], isKnownConnection);
    // In flight or completed-but-not-yet-swapped — either way: hold.
    expect(resolveConnectAction(resolution, undefined, null, 'conn-bastion')).toEqual({
      action: 'hold',
    });
    const existing = session('session-bastion', 'conn-bastion', 'connecting');
    expect(resolveConnectAction(resolution, existing, 'session-bastion', 'conn-bastion')).toEqual({
      action: 'hold',
    });
  });

  it('a connect route whose existing session the hook does not hold still adopts via connect', () => {
    // Remount on a connection route whose session is already alive: the
    // idempotent 7.1 admission returns the same session id.
    const resolution = resolveSessionView('conn-bastion', [], isKnownConnection);
    const existing = session('session-bastion', 'conn-bastion', 'connected');
    expect(resolveConnectAction(resolution, existing, null, null)).toEqual({
      action: 'connect',
      connectionId: 'conn-bastion',
    });
  });

  it('an attached view holds — including a terminal session: restart needs the explicit Connect gesture (11.9)', () => {
    for (const status of ['connected', 'reconnecting', 'error', 'disconnected'] as const) {
      const s = session('session-bastion', 'conn-bastion', status);
      const resolution = resolveSessionView('session-bastion', [s], isKnownConnection);
      expect(resolveConnectAction(resolution, s, 'session-bastion', null)).toEqual({
        action: 'hold',
      });
    }
  });

  it('an unattached view adopts through the idempotent connect (tab switch / remount)', () => {
    const s = session('session-bastion', 'conn-bastion', 'connected');
    const resolution = resolveSessionView('session-bastion', [s], isKnownConnection);
    expect(resolveConnectAction(resolution, s, 'session-jump', 'conn-jump')).toEqual({
      action: 'connect',
      connectionId: 'conn-bastion',
    });
    expect(resolveConnectAction(resolution, s, null, null)).toEqual({
      action: 'connect',
      connectionId: 'conn-bastion',
    });
  });

  it('a stale resolution holds', () => {
    const resolution = resolveSessionView('session-gone', [], () => false);
    expect(resolveConnectAction(resolution, undefined, null, null)).toEqual({ action: 'hold' });
  });
});

describe('resolveConnectAction — failure retry authorization (round 2, findings 3+4)', () => {
  const resolution = resolveSessionView('conn-bastion', [], isKnownConnection);

  it('a Connect gesture newer than the failed initiation re-fires the connect (finding 3)', () => {
    // The failed attempt fired on an organic route arrival (string form
    // of the guard); pressing Connect again produces a NEW gesture
    // signal — it must fire a NEW tauri.connect.
    expect(
      resolveConnectAction(resolution, undefined, null, 'conn-bastion', {
        connectionId: 'conn-bastion',
        nonce: 1,
      })
    ).toEqual({ action: 'connect', connectionId: 'conn-bastion' });
    // Same for an attempt that itself was gesture-authorized: only a
    // gesture the initiation did NOT already consume re-fires.
    expect(
      resolveConnectAction(
        resolution,
        undefined,
        null,
        { connectionId: 'conn-bastion', request: null },
        { connectionId: 'conn-bastion', nonce: 1 }
      )
    ).toEqual({ action: 'connect', connectionId: 'conn-bastion' });
  });

  it('a failed connect never AUTO-retries: without a new gesture the guard holds', () => {
    expect(resolveConnectAction(resolution, undefined, null, 'conn-bastion', null)).toEqual({
      action: 'hold',
    });
    // Even the very signal that authorized the failed attempt (not yet
    // consumed) must not re-fire it — that way persistent failures
    // cannot loop.
    const spent = { connectionId: 'conn-bastion', nonce: 1 };
    expect(
      resolveConnectAction(
        resolution,
        undefined,
        null,
        { connectionId: 'conn-bastion', request: spent },
        spent
      )
    ).toEqual({ action: 'hold' });
  });

  it('a gesture for another connection never authorizes this route', () => {
    expect(
      resolveConnectAction(resolution, undefined, null, 'conn-bastion', {
        connectionId: 'conn-jump',
        nonce: 9,
      })
    ).toEqual({ action: 'hold' });
  });

  it('the attach guard outranks a pending gesture: attached holds (post-connect churn)', () => {
    const existing = session('session-bastion', 'conn-bastion', 'connected');
    expect(
      resolveConnectAction(resolution, existing, 'session-bastion', null, {
        connectionId: 'conn-bastion',
        nonce: 5,
      })
    ).toEqual({ action: 'hold' });
  });
});

describe('createConnectController — failure recovery (round 2, findings 3+4)', () => {
  const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

  function harness() {
    const sessions = new Map<string, SessionInfo>();
    // The hook's connectFn resolves even on failure (it swallows the
    // rejection after setting status 'error'); the STORE is the
    // reliable failure signal, so the mock only adds a session on
    // success — exactly what the real hook does.
    const connect = vi.fn<(connectionId: string) => Promise<void>>(
      () => Promise.resolve()
    );
    const reevaluate = vi.fn();
    const clearRequest = vi.fn();
    const controller = createConnectController({
      connect,
      sessionForConnection: (connectionId) =>
        sessionForConnection(sessions.values(), connectionId),
      requestReevaluation: () => reevaluate(),
      clearRequest: () => clearRequest(),
    });
    return { sessions, connect, reevaluate, clearRequest, controller };
  }

  it('finding 3: a failed connect is retryable — a NEW Connect gesture fires a second connect, an automatic one never fires', async () => {
    const h = harness();
    const resolution = resolveSessionView('conn-bastion', [], isKnownConnection);

    // Fresh transitional route: the connect fires exactly once and
    // rejects (no session is added).
    h.connect.mockRejectedValueOnce(new Error('connection refused'));
    h.controller.evaluate({
      resolution,
      existingSession: undefined,
      hookSessionId: null,
      request: null,
    });
    expect(h.connect).toHaveBeenCalledTimes(1);
    expect(h.connect).toHaveBeenCalledWith('conn-bastion');
    await flush();

    // The attempt settled without a session → the stateful wake-up
    // signal fired (the connect effect's re-evaluation driver)...
    expect(h.reevaluate).toHaveBeenCalledTimes(1);
    // ...but the woken evaluation HOLDS: without a new gesture there
    // is no automatic retry (persistent failures cannot loop).
    h.controller.evaluate({
      resolution,
      existingSession: undefined,
      hookSessionId: null,
      request: null,
    });
    expect(h.connect).toHaveBeenCalledTimes(1);

    // Connect pressed again: a NEW gesture signal (the restartRequest
    // nonce model) must fire a NEW tauri.connect...
    h.controller.evaluate({
      resolution,
      existingSession: undefined,
      hookSessionId: null,
      request: { connectionId: 'conn-bastion', nonce: 2 },
    });
    expect(h.connect).toHaveBeenCalledTimes(2);
    expect(h.connect).toHaveBeenLastCalledWith('conn-bastion');
    // ...and the gesture is consumed (single-shot).
    expect(h.clearRequest).toHaveBeenCalledTimes(1);
  });

  it('finding 4: a route queued behind an in-flight connect connects after the predecessor rejects', async () => {
    const h = harness();
    const resolutionA = resolveSessionView('conn-bastion', [], isKnownConnection);
    const resolutionB = resolveSessionView('conn-jump', [], isKnownConnection);

    // connA's connect starts on its transitional route and stays pending.
    let rejectA!: (reason: Error) => void;
    h.connect.mockImplementationOnce(
      () => new Promise<void>((_, reject) => (rejectA = reject))
    );
    h.controller.evaluate({
      resolution: resolutionA,
      existingSession: undefined,
      hookSessionId: null,
      request: { connectionId: 'conn-bastion', nonce: 1 },
    });
    expect(h.connect).toHaveBeenCalledTimes(1);

    // Navigate to /session/conn-jump while connA is still in flight:
    // the route-id re-arm plus the in-flight gate leave conn-jump
    // QUEUED — no second connect yet, and the queued gesture is not
    // consumed (only connA's own gesture, at its fire, was).
    h.controller.routeChanged();
    h.controller.evaluate({
      resolution: resolutionB,
      existingSession: undefined,
      hookSessionId: null,
      request: { connectionId: 'conn-jump', nonce: 1 },
    });
    expect(h.connect).toHaveBeenCalledTimes(1);
    expect(h.clearRequest).toHaveBeenCalledTimes(1);

    // connA rejects (no session for it) → the wake-up signal fires...
    rejectA(new Error('connection refused'));
    await flush();
    expect(h.reevaluate).toHaveBeenCalledTimes(1);
    expect(h.connect).toHaveBeenCalledTimes(1);

    // ...and the woken evaluation connects the queued route.
    h.controller.evaluate({
      resolution: resolutionB,
      existingSession: undefined,
      hookSessionId: null,
      request: { connectionId: 'conn-jump', nonce: 1 },
    });
    expect(h.connect).toHaveBeenCalledTimes(2);
    expect(h.connect).toHaveBeenLastCalledWith('conn-jump');
    // The queued gesture is consumed by ITS fire.
    expect(h.clearRequest).toHaveBeenCalledTimes(2);
  });

  it('post-connect store churn never re-fires: after a successful connect, re-evaluations hold', async () => {
    const h = harness();
    const resolution = resolveSessionView('conn-bastion', [], isKnownConnection);

    h.connect.mockImplementationOnce((connectionId: string) => {
      h.sessions.set('session-bastion', session('session-bastion', connectionId, 'connected'));
      return Promise.resolve();
    });
    h.controller.evaluate({
      resolution,
      existingSession: undefined,
      hookSessionId: null,
      request: null,
    });
    await flush();
    expect(h.connect).toHaveBeenCalledTimes(1);

    // Every sessionStore write swaps the Map identity while the content
    // keeps naming the same session — the woken/re-run evaluations must
    // hold on content, never re-issue connect IPC.
    const churned = [
      session('session-bastion', 'conn-bastion', 'connected'),
      { ...session('session-bastion', 'conn-bastion', 'connected'), fps: 12 },
      { ...session('session-bastion', 'conn-bastion', 'connected'), fps: 60, latency: 20 },
    ];
    for (const s of churned) {
      h.sessions.set('session-bastion', s);
      h.controller.evaluate({
        resolution,
        existingSession: h.sessions.get('session-bastion'),
        hookSessionId: 'session-bastion',
        request: null,
      });
    }
    expect(h.connect).toHaveBeenCalledTimes(1);
    expect(h.reevaluate).not.toHaveBeenCalled();
  });
});
