import type { NodeId, SessionInfo } from '../types';
import { isTerminalSessionStatus, sessionRoute } from './session-route';

/**
 * Task 8.5/8.6 (Req 10.6, 11.8, 11.9; design.md L379/L396): the ONE
 * shared connect action — the tree (Enter / double-click / Connect
 * menu), the dashboard (double-click / Connect button) and quick
 * connect all consume exactly this core via
 * `src/hooks/useConnectAction.ts`; nothing else may navigate to a
 * session route on its own.
 *
 * Task 8.6 routes by SESSION id. When the connection already has a
 * session — connecting/connected/reconnecting (11.8) or terminal
 * (11.9: 7.1 restart keeps the id) — the action focuses that exact
 * session's tab without starting anything. Only a connection without a
 * session takes the transitional connection-id route; SessionView then
 * connects on mount and replaces the URL with the session id the
 * backend returns (`tauri.connect` resolves to the session id, and 7.1
 * admission makes duplicate connects idempotent).
 */

/**
 * Transitional route for a connection with no session yet — SessionView
 * connects and immediately swaps in `/session/:sessionId`.
 */
export function connectRoute(connectionId: string): string {
  return `/session/${encodeURIComponent(connectionId)}`;
}

/** Collaborators the pure action needs (injected; the hook wires real ones). */
export interface ConnectActionDeps {
  selectNode: (node: NodeId) => void;
  navigate: (path: string) => void;
  /** Live lookup: the existing session for a connection, if any (11.8/11.9). */
  sessionForConnection: (connectionId: string) => SessionInfo | undefined;
  /** Restart a terminal session in its own tab (Req 11.9). */
  requestRestart: (sessionId: string) => void;
  /**
   * Register the fresh-connect gesture (remediation round 2,
   * findings 3+4): the store nonce SessionView's connect effect
   * re-evaluates on, so pressing Connect again on the SAME
   * `/session/:connectionId` route (e.g. after a failed connect) fires
   * a NEW `tauri.connect`.
   */
  requestConnect: (connectionId: string) => void;
}

export type ConnectAction = (node: NodeId) => void;

/**
 * Connect a connection node: select it (so the dashboard the user
 * returns to is scoped to it), then navigate — to the existing
 * session's tab when one exists, else to the transitional connect
 * route. The action never touches sessions — connecting must not
 * disconnect or remove anything (Req 11.6).
 */
export function createConnectAction(deps: ConnectActionDeps): ConnectAction {
  return (node: NodeId) => {
    deps.selectNode(node);
    const existing = deps.sessionForConnection(node.id);
    if (existing) {
      // Focus the exact session's tab (11.8); a terminal session is
      // restarted in this same tab with stable identity (11.9).
      if (isTerminalSessionStatus(existing.status)) {
        deps.requestRestart(existing.id);
      }
      deps.navigate(sessionRoute(existing.id));
      return;
    }
    // Same-tick batching invariant: the store write and the navigation
    // are committed in ONE render (React 18+ automatic batching via
    // createRoot), so SessionView's first connect evaluation already
    // sees the gesture — it fires exactly one connect per press.
    deps.requestConnect(node.id);
    deps.navigate(connectRoute(node.id));
  };
}
