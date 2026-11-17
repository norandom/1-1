import { useState, useCallback, useEffect, useMemo, useRef } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { AlertTriangle } from 'lucide-react';
import { useRdpSession } from '../../hooks/useRdpSession';
import { useClipboard } from '../../hooks/useClipboard';
import { useConnectionStore } from '../../stores/connectionStore';
import { useSessionStore } from '../../stores/sessionStore';
import { SessionCanvas } from './SessionCanvas';
import { SessionToolbar } from './SessionToolbar';
import { PerformanceHUD } from './PerformanceHUD';
import * as tauri from '../../lib/tauri';
import { ctrlAltDelSequence } from '../../lib/input-mapper';
import { normalizeHostProfile } from '../../lib/host-profile';
import { onSessionViewReturn } from '../../lib/idle-preview';
import { physicalViewport } from '../../lib/viewport-size';
import { subscribeSessionEscape } from './canvas-keyboard';
import { resolveSessionView, sessionForConnection } from '../../lib/session-route';
import {
  createConnectController,
  resolveUrlAction,
  type ConnectController,
} from '../../lib/session-attach';
import {
  createTrailingResizeScheduler,
  type ResizeScheduler,
} from '../../lib/resize-debounce';

export function SessionView() {
  const { id } = useParams<{ id: string }>();
  const connectionId = useSessionStore((s) => s.sessions.get(id ?? '')?.connectionId ?? id);
  // A new connection owns fresh input/clipboard/controller state. The initial
  // connection route and its eventual session route intentionally share a key.
  return <SessionContent key={connectionId} />;
}

function SessionContent() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const connections = useConnectionStore((s) => s.connections);
  const sessions = useSessionStore((s) => s.sessions);
  const setActive = useSessionStore((s) => s.setActive);
  const restartRequest = useSessionStore((s) => s.restartRequest);
  const clearRestartRequest = useSessionStore((s) => s.clearRestartRequest);

  const {
    sessionId,
    status,
    fps,
    latency,
    network,
    graphics,
    reconnectAttempts,
    maxReconnectAttempts,
    displayStalled,
    refreshDisplay,
    subscribeToFrames,
    reportFramePresented,
    connect,
    restart,
    disconnect,
    cancelReconnect,
    sendKey,
    sendWheel,
    sendMouse,
  } = useRdpSession(id && sessions.has(id) ? id : undefined);

  const [isFullscreen, setIsFullscreen] = useState(false);
  const [showDisconnectConfirm, setShowDisconnectConfirm] = useState(false);
  const resizeSchedulerRef = useRef<ResizeScheduler | null>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  useClipboard({ sessionId, status });
  const sessionResolution = useSessionStore((s) =>
    sessionId ? s.sessions.get(sessionId) : null
  );

  // Task 8.6 (Req 11.4, design.md L396): the route is `/session/:sessionId`
  // and the connection is resolved THROUGH the session (D1 fix). A
  // connection id on the route is transitional — a fresh connect whose
  // URL is swapped for the backend session id below; anything else is
  // stale (e.g. Back after a tab close) and falls back to `/` (11.5).
  const resolution = useMemo(
    () =>
      resolveSessionView(id, sessions.values(), (cid) =>
        connections.some((c) => c.id === cid)
      ),
    [id, sessions, connections]
  );
  const resolvedConnectionId = resolution.kind === 'stale' ? null : resolution.connectionId;
  const connectionInfo = resolvedConnectionId
    ? connections.find((c) => c.id === resolvedConnectionId)
    : undefined;
  const profile = normalizeHostProfile(connectionInfo?.hostProfile);
  useEffect(() => {
    if (!sessionId || status !== 'connected') return;
    return onSessionViewReturn(window, document, () => {
      // Read current runtime policy on focus. Periodic session snapshots never
      // re-arm this listener or turn idle previews into synthetic activity.
      const state = useSessionStore.getState().sessions.get(sessionId)?.idlePreviewState;
      if (state === 'live' || state === 'suppressed' || state === 'preview') {
        void tauri.resumeDisplay(sessionId).catch((error) => console.warn('Resume display failed', error));
      }
    });
  }, [sessionId, status]);
  useEffect(() => {
    if (sessionId === null || profile.resizeMode === 'fixed') return;
    const viewport = viewportRef.current;
    const initialSize = viewport
      ? physicalViewport(viewport.offsetWidth, viewport.offsetHeight, window.devicePixelRatio)
      : undefined;
    const scheduler = createTrailingResizeScheduler((width, height) => {
      tauri.resizeSession(sessionId, width, height).catch(() => {});
    }, undefined, () => {
      // Recheck the committed preference at emit time as well as effect cleanup.
      const current = useConnectionStore.getState().connections.find((c) => c.id === resolvedConnectionId);
      return !!current && normalizeHostProfile(current.hostProfile).resizeMode === 'dynamic';
    }, initialSize);
    resizeSchedulerRef.current = scheduler;
    return () => {
      scheduler.cancel();
      if (resizeSchedulerRef.current === scheduler) resizeSchedulerRef.current = null;
    };
  }, [sessionId, resolvedConnectionId, profile.resizeMode]);

  // The existing session for a transitional route's connection, if any
  // — the input the pure swap/guard decisions below consume.
  const existingForConnection = useMemo(
    () =>
      resolution.kind === 'connect'
        ? sessionForConnection(sessions.values(), resolution.connectionId)
        : undefined,
    [resolution, sessions]
  );

  // URL decisions (remediation finding 1, Req 11.4/11.5): the stale
  // redirect AND the transitional-route → session-route swap are ONE
  // pure decision — `resolveUrlAction` (node-tested). The swap fires as
  // soon as the connect initiated on this route has completed (the
  // store and this hook hold the backend's session id), so
  // `/session/:sessionId` is THE session route from the first connect
  // on and the active-tab underline resolves while viewing the session
  // (11.1). This replaces the previous dead swap effect, whose
  // condition could never hold inside the `kind: 'view'` branch.
  useEffect(() => {
    const urlAction = resolveUrlAction(resolution, existingForConnection, sessionId);
    if (urlAction.action !== 'replace') return;
    navigate(urlAction.route, { replace: true });
  }, [resolution, existingForConnection, sessionId, navigate]);

  // The session runtime owns channels across view changes. Route attachment
  // is local; the controller starts fresh sessions or explicit restarts.
  const connectRequest = useSessionStore((s) => s.connectRequest);
  // Round 2 (findings 3+4): the STATEFUL wake-up signal in the connect
  // effect's deps — the in-repo `restartRequest` nonce pattern. When an
  // attempt settles without a session (the hook's connectFn resolves
  // even on failure — it swallows the rejection after setting status
  // 'error', so the store is the failure signal), the controller bumps
  // this nonce and the effect re-evaluates: a route queued behind the
  // failed attempt (finding 4), or a Connect press issued while it was
  // in flight (finding 3), connects on the woken pass. A woken pass
  // without a NEW gesture holds — persistent failures never auto-retry.
  const [connectWake, setConnectWake] = useState(0);
  // Latest-ref indirection (written in effects only): the controller is
  // created once, so a changed `connect` identity (defensively — its
  // useCallback deps are stable zustand actions) must not strand a
  // stale closure in it. All three effects below run in declaration
  // order within a commit's flush: ref refresh → (once) controller
  // creation → re-arm → evaluate.
  const connectRef = useRef(connect);
  const restartRef = useRef(restart);
  const connectControllerRef = useRef<ConnectController | null>(null);
  const measuredSize = useCallback((connectionId: string) => {
    const saved = useConnectionStore.getState().connections.find((c) => c.id === connectionId);
    const viewport = viewportRef.current;
    if (!viewport || normalizeHostProfile(saved?.hostProfile).resizeMode === 'fixed') return undefined;
    return physicalViewport(viewport.offsetWidth, viewport.offsetHeight, window.devicePixelRatio);
  }, []);
  useEffect(() => {
    connectRef.current = connect;
    restartRef.current = restart;
  });
  useEffect(() => {
    if (connectControllerRef.current == null) {
      connectControllerRef.current = createConnectController({
        connect: (connectionId) => connectRef.current(connectionId, measuredSize(connectionId)),
        restart: (connectionId) => restartRef.current(connectionId, measuredSize(connectionId)),
        sessionForConnection: (connectionId) =>
          sessionForConnection(
            useSessionStore.getState().sessions.values(),
            connectionId
          ),
        requestReevaluation: () => setConnectWake((n) => n + 1),
        clearRequest: () => useSessionStore.getState().clearConnectRequest(),
      });
    }
  }, [measuredSize]);
  // A new route id may legitimately initiate its own connect: re-arm
  // the controller's once-guard whenever the route id changes. Declared
  // BEFORE the connect effect so the re-arm runs first on the same
  // commit.
  useEffect(() => {
    connectControllerRef.current?.routeChanged();
  }, [id]);
  useEffect(() => {
    connectControllerRef.current?.evaluate({
      resolution,
      existingSession: existingForConnection,
      hookSessionId: sessionId,
      request: connectRequest,
    });
  }, [resolution, existingForConnection, sessionId, connectRequest, connectWake]);

  // Req 11.9: a Connect requested for this terminal session while its
  // tab is already open restarts it here — same tab, same session id
  // (7.1 restart keeps identity). Remediation finding 5: the request is
  // single-shot — if navigation landed anywhere other than the
  // requested session (another tab, a transitional route, a stale id),
  // it is CONSUMED here, so a later mere tab selection can never
  // auto-restart the terminal session: selection is not Connect.
  // Same-tick batching invariant (round 2 pin): correctness relies on
  // `requestRestart` + `navigate` in createConnectAction landing in ONE
  // committed render — React 18+ automatic batching via createRoot
  // batches the zustand store write and the router navigation raised in
  // the same event handler. Were they split across renders, this
  // effect's first run could miss the request (the hook-unattached view
  // adopt-connects first), and the request arriving on the second
  // render would fire a second, redundant connect.
  useEffect(() => {
    if (!restartRequest) return;
    if (
      resolution.kind !== 'view' ||
      resolution.sessionId !== restartRequest.sessionId
    ) {
      clearRestartRequest();
      return;
    }
    clearRestartRequest();
    connectControllerRef.current?.runImmediate(resolution.connectionId);
  }, [restartRequest, resolution, clearRestartRequest]);

  // Keep the store's active session (StatusBar) aligned with the route.
  useEffect(() => {
    if (resolution.kind === 'view') setActive(resolution.sessionId);
  }, [resolution, setActive]);

  const handleFullscreen = useCallback(() => {
    if (!document.fullscreenElement) {
      document.documentElement.requestFullscreen();
      setIsFullscreen(true);
    } else {
      document.exitFullscreen();
      setIsFullscreen(false);
    }
  }, []);

  const handleCtrlAltDel = useCallback(() => {
    if (!sessionId) return;
    // Send Ctrl+Alt+Del key sequence
    for (const key of ctrlAltDelSequence()) {
      tauri.sendKeyEvent(sessionId, key).catch(() => {});
    }
  }, [sessionId]);

  const handleScreenshot = useCallback(() => {
    const canvas = document.querySelector('canvas');
    if (!canvas) return;
    const link = document.createElement('a');
    link.download = `rdp-screenshot-${Date.now()}.png`;
    link.href = canvas.toDataURL();
    link.click();
  }, []);

  const handleDisconnect = useCallback(async () => {
    await disconnect();
    navigate('/');
  }, [disconnect, navigate]);

  const handleCancelReconnect = useCallback(async () => {
    await cancelReconnect();
    await disconnect();
    navigate('/');
  }, [cancelReconnect, disconnect, navigate]);

  const handleResize = useCallback(
    (width: number, height: number) => {
      if (!sessionId || profile.resizeMode === 'fixed') return;
      resizeSchedulerRef.current?.schedule(width, height);
    },
    [sessionId, profile.resizeMode]
  );

  // Escape key to show disconnect confirm
  useEffect(() => {
    if (status !== 'connected') return;
    return subscribeSessionEscape(window, () => setShowDisconnectConfirm(true));
  }, [status]);

  // Resolved through the session (D1): the route id never names the
  // connection directly once a session exists.
  const hostname = connectionInfo?.host ?? 'Unknown';
  const surfaceWidth = sessionResolution?.width
    ?? connectionInfo?.displayWidth
    ?? 1920;
  const surfaceHeight = sessionResolution?.height
    ?? connectionInfo?.displayHeight
    ?? 1080;
  const displayResolution = `${surfaceWidth}x${surfaceHeight}`;

  return (
    <div className="flex flex-col h-full relative">
      {/* Task 8.6 (Req 11.1): the tab strip is hoisted to App above the
          routes — it no longer renders inside the session view. */}

      <div ref={viewportRef} className="relative min-h-0 flex-1 overflow-hidden">
        {/* Canvas */}
        <SessionCanvas
          displayMode={profile.displayMode}
          inputBlocked={showDisconnectConfirm}
          lastError={sessionResolution?.lastError}
          overlays={
            <>
              {/* Toolbar */}
              <SessionToolbar
                hostname={hostname}
                resolution={displayResolution}
                fps={fps}
                latency={latency}
                onFullscreen={handleFullscreen}
                onCtrlAltDel={handleCtrlAltDel}
                onScreenshot={handleScreenshot}
                onDisconnect={() => setShowDisconnectConfirm(true)}
                isFullscreen={isFullscreen}
              />

              {/* Performance HUD */}
              <PerformanceHUD
                fps={fps}
                latency={latency}
                network={network}
                graphics={graphics}
                resolution={displayResolution}
              />
            </>
          }
          displayStalled={displayStalled}
          onRefreshDisplay={refreshDisplay}
          key={sessionId ?? id}
          subscribeToFrames={subscribeToFrames}
          status={status}
          reconnectAttempts={reconnectAttempts}
          maxReconnectAttempts={maxReconnectAttempts}
          surfaceWidth={surfaceWidth}
          surfaceHeight={surfaceHeight}
          onFramePresented={reportFramePresented}
          onKeyDown={sendKey}
          onKeyUp={sendKey}
          onMouseMove={(e, rect) => sendMouse(e, 'move', rect)}
          onMouseDown={(e, rect) => sendMouse(e, 'down', rect)}
          onMouseUp={(e, rect) => sendMouse(e, 'up', rect)}
          onWheel={(e, rect) => sendWheel(e, rect)}
          onResize={handleResize}
        />
      </div>

      {/* Cancel Reconnect button */}
      {status === 'reconnecting' && (
        <div className="absolute bottom-6 left-1/2 -translate-x-1/2 z-50">
          <button
            onClick={handleCancelReconnect}
            className="px-5 py-2.5 text-sm font-medium rounded-lg text-white bg-[var(--color-danger)] hover:bg-[var(--color-danger)]/80 transition-colors shadow-lg"
          >
            Cancel Reconnect
          </button>
        </div>
      )}

      {/* Disconnect confirmation */}
      <AnimatePresence>
        {showDisconnectConfirm && (
          <motion.div
            data-local-input
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="absolute inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
            onClick={() => setShowDisconnectConfirm(false)}
          >
            <motion.div
              initial={{ scale: 0.9, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              exit={{ scale: 0.9, opacity: 0 }}
              onClick={(e) => e.stopPropagation()}
              className="w-full max-w-sm rounded-2xl bg-[var(--color-surface-1)] border border-[var(--color-border)] shadow-2xl p-6 text-center"
            >
              <div className="flex items-center justify-center w-12 h-12 rounded-xl bg-[var(--color-warning)]/10 mx-auto mb-4">
                <AlertTriangle size={24} className="text-[var(--color-warning)]" />
              </div>
              <h3 className="text-base font-semibold text-[var(--color-text-primary)] mb-1">
                Disconnect Session?
              </h3>
              <p className="text-xs text-[var(--color-text-muted)] mb-5">
                This will close your connection to {hostname}. Any unsaved work on the remote machine will remain open.
              </p>
              <div className="flex items-center gap-3 justify-center">
                <button
                  onClick={() => setShowDisconnectConfirm(false)}
                  className="px-4 py-2 text-sm font-medium rounded-lg text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-3)] transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handleDisconnect}
                  className="px-4 py-2 text-sm font-medium rounded-lg text-white bg-[var(--color-danger)] hover:bg-[var(--color-danger)]/80 transition-colors"
                >
                  Disconnect
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
