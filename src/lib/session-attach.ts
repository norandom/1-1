/**
 * Task 8.6 remediation round 1 (Req 11.4/11.8/11.9, design.md L396):
 * the PURE SessionView attach decisions — when the transitional
 * `/session/:connectionId` URL must be swapped (replace-navigated) for
 * the `/session/:sessionId` route the backend returned, and when the
 * connect effect may fire `tauri.connect` IPC at all. SessionView stays
 * render-thin: it executes exactly these decisions (pinned by
 * `src/lib/__tests__/session-attach.test.ts`).
 *
 * Task 8.6 remediation round 2 (findings 3+4): the once-only connect
 * guard had no failure reset — a rejected connect left its route
 * silently dead and a route queued behind an in-flight connect never
 * fired after the predecessor failed. `createConnectController` below
 * is the node-tested connect-effect body: a NEW Connect gesture
 * (sessionStore `connectRequest`, the `restartRequest` nonce pattern)
 * re-fires a failed connect, a settle-time wake-up nonce re-evaluates
 * so queued routes connect after a predecessor failure, and neither
 * ever weakens the round-1 loop guard.
 */
import type { SessionInfo } from '../types';
import type { SessionViewResolution } from './session-route';
import { sessionRoute } from './session-route';

/** What SessionView must do with the URL for the current resolution. */
export type SessionUrlAction =
  | { action: 'none' }
  /** Replace the URL without adding a history entry. */
  | { action: 'replace'; route: string };

/**
 * The URL swap (finding 1, Req 11.4): a transitional connect route is
 * swapped for the session route — replace-navigate — as soon as the
 * store holds the connection's session AND the hook holds it, i.e. the
 * connect initiated on this route has completed. A view resolution
 * never swaps (the URL already names the session); a stale resolution
 * redirects to the dashboard/start view (Req 11.5).
 */
export function resolveUrlAction(
  resolution: SessionViewResolution,
  existingSession: SessionInfo | undefined,
  hookSessionId: string | null
): SessionUrlAction {
  if (resolution.kind === 'stale') {
    return { action: 'replace', route: '/' };
  }
  if (resolution.kind === 'view') {
    return { action: 'none' };
  }
  if (existingSession && hookSessionId === existingSession.id) {
    return { action: 'replace', route: sessionRoute(existingSession.id) };
  }
  return { action: 'none' };
}

/** Whether SessionView may fire `connect` IPC for the resolution. */
export type SessionConnectAction =
  | { action: 'connect'; connectionId: string }
  | {
      /** No IPC: attached, already initiated on this route, or
       * post-connect store churn. */
      action: 'hold';
    };

/**
 * A user Connect gesture for a connection without a session (remediation
 * round 2, findings 3+4) — sessionStore's `connectRequest`. A NEW object
 * per press (the `restartRequest` nonce pattern), so a re-press on the
 * SAME transitional route re-triggers what a stable memoized resolution
 * never would.
 */
export interface ConnectRequestSignal {
  connectionId: string;
  nonce: number;
}

/**
 * What the connect effect last fired for, and the exact gesture signal
 * that authorized it (`request: null` = organic route arrival). The
 * signal OBJECT is recorded (not its nonce): the store counter restarts
 * after a consume, so only identity reliably says "a gesture the
 * initiation has not already consumed".
 */
export interface ConnectInitiation {
  connectionId: string;
  request: ConnectRequestSignal | null;
}

/** Normalize the legacy string form of the initiation guard. */
function toInitiation(
  initiated: string | ConnectInitiation | null
): ConnectInitiation | null {
  if (initiated === null) return null;
  return typeof initiated === 'string'
    ? { connectionId: initiated, request: null }
    : initiated;
}

/**
 * The reconnect loop guard (finding 2): on a transitional connect route
 * the connect must fire exactly ONCE. After it completes, every
 * sessionStore write (the session-info listener and the 1 Hz fps
 * interval) swaps the Map identity, which re-renders and re-identifies
 * the resolution — the decision below holds on CONTENT (the store and
 * the hook hold the connection's session) and on the recorded
 * initiation, so store-identity churn can never re-trigger IPC.
 *
 * Remediation round 2 (findings 3+4): a FAILED connect used to leave
 * the once-guard armed with no way back — no session was added, no
 * listener/interval started, and nothing re-ran the effect, so
 * pressing Connect again (same path, stable memoized resolution) was
 * silently dead. The guard now yields to a NEWER Connect gesture: a
 * `connectRequest` whose signal object the initiation did not already
 * consume re-fires the connect (identity beats the store's
 * clear-and-restart nonce). The wake-up itself never authorizes
 * anything — without a new gesture a settled-without-session attempt
 * still HOLDS, so persistent failures cannot auto-retry in a loop.
 *
 * View resolutions: attached (including a terminal session — Req 11.9
 * restarts only on an explicit Connect gesture, never on navigation)
 * holds — and that attach guard OUTRANKS any pending gesture;
 * unattached adopts via the idempotent 7.1 admission, which returns
 * the SAME session id. NOTE: the adopt connect does NOT re-attach the
 * frame channel — the backend reuse branch returns before attaching
 * it, and frames keep flowing only through the ORIGINAL connect's
 * channel closure writing into the shared `frameListenersRef` set
 * (useRdpSession.ts) — fragile, flagged in review.
 */
export function resolveConnectAction(
  resolution: SessionViewResolution,
  existingSession: SessionInfo | undefined,
  hookSessionId: string | null,
  initiatedConnectionId: string | ConnectInitiation | null,
  connectRequest: ConnectRequestSignal | null = null
): SessionConnectAction {
  switch (resolution.kind) {
    case 'stale':
      return { action: 'hold' };
    case 'view':
      return hookSessionId === resolution.sessionId
        ? { action: 'hold' }
        : { action: 'connect', connectionId: resolution.connectionId };
    case 'connect': {
      if (existingSession && hookSessionId === existingSession.id) {
        return { action: 'hold' };
      }
      const initiated = toInitiation(initiatedConnectionId);
      const routeId = resolution.connectionId;
      const initiatedForRoute =
        initiated?.connectionId === routeId ? initiated : null;
      const requestForRoute =
        connectRequest?.connectionId === routeId ? connectRequest : null;
      if (requestForRoute && requestForRoute !== initiatedForRoute?.request) {
        return { action: 'connect', connectionId: routeId };
      }
      if (initiatedForRoute) {
        return { action: 'hold' };
      }
      return { action: 'connect', connectionId: routeId };
    }
  }
}

/** Collaborators the connect controller needs (injected; SessionView wires real ones). */
export interface ConnectControllerDeps {
  /** The hook's connect IPC. It RESOLVES even on failure (the hook
   * swallows the rejection after setting status 'error'), so failure is
   * detected via `sessionForConnection` at settle time; a real
   * rejection is tolerated the same way. */
  connect: (connectionId: string) => Promise<void>;
  /** Live lookup: the existing session for a connection, if any — the
   * settle-time failure signal. */
  sessionForConnection: (connectionId: string) => SessionInfo | undefined;
  /**
   * Findings 3+4: bump the STATEFUL wake-up signal in the connect
   * effect's deps (the in-repo `restartRequest` nonce pattern) — a
   * settled-without-session attempt must re-run the evaluation so a
   * route queued behind it (finding 4), or a Connect press issued
   * while it was in flight (finding 3), connects on the woken pass.
   */
  requestReevaluation: () => void;
  /** Consume a handled connect request (single-shot, restartRequest model). */
  clearRequest: () => void;
}

/** One connect-effect evaluation — the committed store/hook state. */
export interface ConnectEvaluationInput {
  resolution: SessionViewResolution;
  existingSession: SessionInfo | undefined;
  hookSessionId: string | null;
  request: ConnectRequestSignal | null;
}

/**
 * The SessionView connect-effect body (remediation round 2, findings
 * 3+4), node-tested here so SessionView stays render-thin. Owns the
 * once-per-gesture initiation guard and the in-flight gate; every
 * connect-effect dep change (resolution content, the hook's session,
 * the store's `connectRequest`, the wake-up nonce) calls `evaluate`.
 *
 * Reset semantics: the guard is re-armed per route arrival
 * (`routeChanged`). On failure the guard is NOT reset — that would let
 * the wake-up itself (or any unrelated store churn) re-fire the
 * connect and loop on persistent failure; instead a failed initiation
 * yields to the NEXT Connect gesture by signal-object identity. A
 * gesture pressed while a connect is in flight stays armed: the busy
 * gate leaves it unconsumed and the settle-time wake-up evaluates it.
 */
export interface ConnectController {
  /** The connect-effect body; run on every dep change. */
  evaluate(evaluation: ConnectEvaluationInput): void;
  /** Route id changed: re-arm the once-per-route initiation. */
  routeChanged(): void;
  /**
   * Fire outside the once-guard (the Req 11.9 restart path); gates on
   * in-flight and settles like an ordinary attempt.
   */
  runImmediate(connectionId: string): void;
  /** Whether a connect is in flight. */
  isBusy(): boolean;
}

export function createConnectController(
  deps: ConnectControllerDeps
): ConnectController {
  let initiated: ConnectInitiation | null = null;
  let busy = false;

  const settle = (connectionId: string) => {
    busy = false;
    if (deps.sessionForConnection(connectionId)) return;
    // Settled without a session: the attempt failed. Wake the effect —
    // never auto-retry (resolveConnectAction holds without a new
    // gesture, pinning the loop guard).
    deps.requestReevaluation();
  };

  const settleAfter = (connectionId: string) => {
    deps.connect(connectionId)
      .catch(() => {})
      .then(() => settle(connectionId));
  };

  return {
    evaluate({ resolution, existingSession, hookSessionId, request }) {
      if (busy) return;
      const decision = resolveConnectAction(
        resolution,
        existingSession,
        hookSessionId,
        initiated,
        request
      );
      const routeConnectionId =
        resolution.kind === 'connect' ? resolution.connectionId : null;
      if (decision.action !== 'connect') {
        // Consume-on-miss (single-shot, restartRequest model): a
        // gesture this evaluation cannot act on is spent here.
        if (request && request.connectionId !== routeConnectionId) {
          deps.clearRequest();
        }
        return;
      }
      if (request && request.connectionId === decision.connectionId) {
        deps.clearRequest();
      }
      initiated = {
        connectionId: decision.connectionId,
        request:
          request && request.connectionId === decision.connectionId
            ? request
            : null,
      };
      busy = true;
      settleAfter(decision.connectionId);
    },
    routeChanged() {
      initiated = null;
    },
    runImmediate(connectionId) {
      if (busy) return;
      busy = true;
      settleAfter(connectionId);
    },
    isBusy() {
      return busy;
    },
  };
}
