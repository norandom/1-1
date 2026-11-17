import { useState, useEffect, useRef, useCallback } from 'react';
import type { SessionStatus } from '../types';
import type { ViewportSize } from '../lib/viewport-size';
import { useSessionStore } from '../stores/sessionStore';
import * as tauri from '../lib/tauri';
import { connectSession, subscribeSessionFrames, reportSessionFramePresented, refreshSessionDisplay, type FrameListener } from '../lib/session-runtime';
import {
  mapKeyEvent,
  mapMouseEvent,
  normalizeWheelDelta,
  type MouseSurfaceBounds,
} from '../lib/input-mapper';


// Views attach to an existing session by ID immediately. Only a fresh Connect
// or explicit Restart invokes the session runtime; unmount detaches the canvas,
// while the session keeps its channel, latest image and status subscription.
export function useRdpSession(viewSessionId?: string) {
  const [connectedSessionId, setConnectedSessionId] = useState<string | null>(null);
  const [connectStatus, setConnectStatus] = useState<SessionStatus>('disconnected');
  const sessionId = viewSessionId ?? connectedSessionId;
  const session = useSessionStore((s) => sessionId ? s.sessions.get(sessionId) : undefined);
  const sessionIdRef = useRef<string | null>(null);
  const pendingMouseMoveRef = useRef<ReturnType<typeof mapMouseEvent> | null>(null);
  const mouseMoveScheduledRef = useRef(false);
  const removeSession = useSessionStore((s) => s.removeSession);

  useEffect(() => {
    sessionIdRef.current = sessionId;
    pendingMouseMoveRef.current = null;
    return () => { sessionIdRef.current = null; pendingMouseMoveRef.current = null; };
  }, [sessionId]);

  const subscribeToFrames = useCallback((listener: FrameListener) => {
    return sessionId ? subscribeSessionFrames(sessionId, listener) : () => {};
  }, [sessionId]);

  const reportFramePresented = useCallback(() => {
    if (sessionId) reportSessionFramePresented(sessionId);
  }, [sessionId]);

  const open = useCallback(async (connectionId: string, restart: boolean, initialSize?: ViewportSize) => {
    setConnectStatus('connecting');
    try {
      const sid = await connectSession(connectionId, restart, initialSize);
      // A close can finish while the status listener is being registered.
      if (useSessionStore.getState().sessions.has(sid)) setConnectedSessionId(sid);
    } catch (error) {
      console.error('Connection failed:', error);
      setConnectStatus('error');
    }
  }, []);
  const connect = useCallback((connectionId: string, initialSize?: ViewportSize) => open(connectionId, false, initialSize), [open]);
  const restart = useCallback((connectionId: string, initialSize?: ViewportSize) => open(connectionId, true, initialSize), [open]);

  const disconnect = useCallback(async () => {
    if (sessionId) {
      await tauri.disconnect(sessionId).catch((error) => console.error('Disconnect error:', error));
      removeSession(sessionId);
    }
    setConnectedSessionId(null);
    setConnectStatus('disconnected');
  }, [sessionId, removeSession]);

  const cancelReconnect = useCallback(async () => {
    if (sessionId) await tauri.cancelReconnect(sessionId).catch((error) => console.error('Cancel reconnect error:', error));
  }, [sessionId]);

  const sendKey = useCallback((event: globalThis.KeyboardEvent) => {
    if (!sessionId) return;
    const mapped = mapKeyEvent(event);
    if (mapped.keyCode !== 0) tauri.sendKeyEvent(sessionId, mapped).catch(() => {});
  }, [sessionId]);

  const sendMouse = useCallback((event: globalThis.MouseEvent, type: 'move' | 'down' | 'up' | 'scroll', bounds?: MouseSurfaceBounds) => {
    const currentSessionId = sessionIdRef.current;
    if (!currentSessionId) return;
    const mapped = mapMouseEvent(event, type, bounds);
    if (type !== 'move') {
      tauri.sendMouseEvent(currentSessionId, mapped).catch(() => {});
      return;
    }
    pendingMouseMoveRef.current = mapped;
    if (mouseMoveScheduledRef.current) return;
    mouseMoveScheduledRef.current = true;
    requestAnimationFrame(() => {
      mouseMoveScheduledRef.current = false;
      const nextMove = pendingMouseMoveRef.current;
      pendingMouseMoveRef.current = null;
      if (nextMove && sessionIdRef.current === currentSessionId) {
        tauri.sendMouseEvent(currentSessionId, nextMove).catch(() => {});
      }
    });
  }, []);

  const sendWheel = useCallback((event: globalThis.WheelEvent, bounds?: MouseSurfaceBounds) => {
    const currentSessionId = sessionIdRef.current;
    if (!currentSessionId) return;
    const mapped = mapMouseEvent(event as unknown as globalThis.MouseEvent, 'scroll', bounds);
    mapped.scrollDelta = normalizeWheelDelta(event.deltaY, event.deltaMode);
    mapped.eventType = 'scroll';
    tauri.sendMouseEvent(currentSessionId, mapped).catch(() => {});
  }, []);

  return {
    displayStalled: session?.displayStalled ?? false,
    refreshDisplay: () => { if (sessionId) refreshSessionDisplay(sessionId); },
    sessionId,
    status: session?.status ?? connectStatus,
    fps: session?.fps ?? 0,
    latency: session?.latency ?? 0,
    bandwidth: session?.bandwidth ?? 0,
    network: session?.network,
    graphics: session?.graphics,
    reconnectAttempts: session?.reconnectAttempts ?? 0,
    maxReconnectAttempts: session?.maxReconnectAttempts ?? 5,
    subscribeToFrames, reportFramePresented, connect, restart, disconnect, cancelReconnect,
    sendKey, sendMouse, sendWheel,
  };
}
