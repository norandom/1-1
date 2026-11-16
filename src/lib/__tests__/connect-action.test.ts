import { describe, expect, it, vi } from 'vitest';

import type { NodeId, SessionInfo, SessionStatus } from '../../types';
import { connectRoute, createConnectAction } from '../connect-action';

// Task 8.6 (Req 11.8/11.9, design.md L396): the ONE shared connect
// action now routes by SESSION id. When the connection already has a
// session (connecting/connected/reconnecting — 11.8; terminal — 11.9
// restart in the same tab) the action focuses that session's tab; only a
// connection without any session takes the transitional connection-id
// route, which SessionView immediately replaces with the session id the
// backend returns (`tauri.connect` → session id, 7.1 idempotent).

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

describe('connectRoute (transitional)', () => {
  it('a connection without a session starts at its connection route; SessionView swaps in the backend session id', () => {
    expect(connectRoute('conn-bastion')).toBe('/session/conn-bastion');
  });
});

describe('createConnectAction (Req 11.8/11.9)', () => {
  const conn = (id: string): NodeId => ({ kind: 'connection', id });

  function makeAction(existing?: SessionInfo) {
    const selectNode = vi.fn();
    const navigate = vi.fn();
    const requestRestart = vi.fn();
    const requestConnect = vi.fn();
    const connect = createConnectAction({
      selectNode,
      navigate,
      sessionForConnection: (connectionId) =>
        existing && existing.connectionId === connectionId ? existing : undefined,
      requestRestart,
      requestConnect,
    });
    return { selectNode, navigate, requestRestart, requestConnect, connect };
  }

  it('selects the node first, then navigates — order preserved from 8.5', () => {
    const { selectNode, navigate, connect } = makeAction();
    connect(conn('conn-bastion'));
    expect(selectNode).toHaveBeenCalledWith(conn('conn-bastion'));
    expect(navigate).toHaveBeenCalledTimes(1);
    expect(selectNode.mock.invocationCallOrder[0]).toBeLessThan(
      navigate.mock.invocationCallOrder[0]
    );
  });

  it.each(['connecting', 'connected', 'reconnecting'] as const)(
    'an existing %s session: focus that exact session tab, start nothing new (11.8)',
    (status) => {
      const { navigate, requestRestart, connect } = makeAction(
        session('session-bastion', 'conn-bastion', status)
      );
      connect(conn('conn-bastion'));
      expect(navigate).toHaveBeenCalledWith('/session/session-bastion');
      expect(navigate).toHaveBeenCalledTimes(1);
      expect(requestRestart).not.toHaveBeenCalled();
    }
  );

  it.each(['disconnected', 'error'] as const)(
    'a terminal %s session: restart targets the SAME tab — identity stays stable (11.9)',
    (status) => {
      const { navigate, requestRestart, connect } = makeAction(
        session('session-bastion', 'conn-bastion', status)
      );
      connect(conn('conn-bastion'));
      expect(requestRestart).toHaveBeenCalledWith('session-bastion');
      expect(navigate).toHaveBeenCalledWith('/session/session-bastion');
    }
  );

  it('a connection without any session takes the transitional connection route', () => {
    const { navigate, connect } = makeAction(undefined);
    connect(conn('conn-jump'));
    expect(navigate).toHaveBeenCalledWith('/session/conn-jump');
  });

  it('a fresh connect registers the gesture signal before navigating (round 2, findings 3+4)', () => {
    const { navigate, requestConnect, connect } = makeAction(undefined);
    connect(conn('conn-jump'));
    // The store nonce is what lets SessionView re-fire a connect when
    // Connect is pressed again on the SAME transitional route (a failed
    // connect must be retryable), and it must land in the same tick as
    // the navigation (see the batching note in connect-action.ts).
    expect(requestConnect).toHaveBeenCalledWith('conn-jump');
    expect(requestConnect.mock.invocationCallOrder[0]).toBeLessThan(
      navigate.mock.invocationCallOrder[0]
    );
  });

  it('focusing an existing session never registers a fresh-connect gesture', () => {
    const existing = session('session-bastion', 'conn-bastion', 'connected');
    const { requestConnect, connect } = makeAction(existing);
    connect(conn('conn-bastion'));
    expect(requestConnect).not.toHaveBeenCalled();
  });

  it('only selects and navigates — connecting never disconnects or removes sessions (11.6)', () => {
    const { selectNode, navigate, connect } = makeAction();
    connect(conn('conn-web-prod'));
    expect(selectNode.mock.calls).toHaveLength(1);
    expect(navigate.mock.calls).toHaveLength(1);
  });
});
