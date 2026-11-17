import { Channel } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { SessionInfo } from '../types';
import { useSessionStore } from '../stores/sessionStore';
import { compositeFrameRects, type CompositingSurface } from './canvas-compositor';
import { parseFramePacket, toFrameBytes, type FramePayload } from './frame-protocol';
import * as tauri from './tauri';

type FrameListener = (frame: Uint8Array) => void;
interface SessionRuntime {
  disposed: boolean;
  channel: Channel<FramePayload> | null;
  listeners: Set<FrameListener>;
  surface: CompositingSurface | null;
  unlisten: (() => void) | null;
  timer: ReturnType<typeof setInterval> | null;
  frames: number;
  bytes: number;
}

// Transports belong to open sessions, not mounted React views. Each session
// retains one composed image; there is no replay queue of incoming packets.
const runtimes = new Map<string, SessionRuntime>();
const pending = new Map<string, Promise<string>>();

function dispose(runtime: SessionRuntime, clearListeners = true) {
  if (runtime.disposed) return;
  runtime.disposed = true;
  runtime.unlisten?.();
  if (runtime.timer) clearInterval(runtime.timer);
  runtime.channel = null;
  runtime.surface = null;
  if (clearListeners) runtime.listeners.clear();
}

useSessionStore.subscribe((state, previous) => {
  for (const id of previous.sessions.keys()) {
    if (state.sessions.has(id)) continue;
    const runtime = runtimes.get(id);
    if (runtime) dispose(runtime);
    runtimes.delete(id);
  }
});

function snapshot(surface: CompositingSurface): Uint8Array {
  // Existing kind-1 packet layout; used only when attaching a new canvas.
  const bytes = new Uint8Array(15 + surface.pixels.length);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, 1);
  view.setUint16(1, surface.width, true);
  view.setUint16(3, surface.height, true);
  view.setUint16(5, 1, true);
  view.setUint16(11, surface.width, true);
  view.setUint16(13, surface.height, true);
  bytes.set(surface.pixels, 15);
  return bytes;
}

export function subscribeSessionFrames(sessionId: string, listener: FrameListener): () => void {
  const runtime = runtimes.get(sessionId);
  if (!runtime || runtime.disposed) return () => {};
  runtime.listeners.add(listener);
  if (runtime.surface) listener(snapshot(runtime.surface));
  return () => { runtime.listeners.delete(listener); };
}

export function reportSessionFramePresented(sessionId: string) {
  const runtime = runtimes.get(sessionId);
  if (runtime) runtime.frames++;
}

export function connectSession(connectionId: string, restart = false): Promise<string> {
  const inflight = pending.get(connectionId);
  if (inflight) return inflight;
  const existing = [...useSessionStore.getState().sessions.values()]
    .find((session) => session.connectionId === connectionId);
  // Navigation never restarts a terminal session either. Only an explicit
  // restart gesture can replace the transport of an existing tab.
  if (existing && (!restart || ['connecting', 'connected', 'reconnecting'].includes(existing.status))) {
    return Promise.resolve(existing.id);
  }
  const attempt = startSession(connectionId, existing).finally(() => pending.delete(connectionId));
  pending.set(connectionId, attempt);
  return attempt;
}

async function startSession(connectionId: string, existing?: SessionInfo): Promise<string> {
  const old = existing ? runtimes.get(existing.id) : undefined;
  const listeners = old?.listeners ?? new Set<FrameListener>();
  if (old) dispose(old, false);
  const runtime: SessionRuntime = {
    disposed: false, channel: null, listeners, surface: null,
    unlisten: null, timer: null, frames: 0, bytes: 0,
  };
  // Publish ownership before restart awaits IPC, so close/delete can cancel it.
  if (existing) runtimes.set(existing.id, runtime);
  let id: string | null = null;
  try {
    runtime.channel = new Channel<FramePayload>((payload) => {
      if (runtime.disposed) return;
      const bytes = toFrameBytes(payload);
      try {
        const packet = parseFramePacket(bytes);
        if (!runtime.surface || runtime.surface.width !== packet.surfaceWidth || runtime.surface.height !== packet.surfaceHeight) {
          runtime.surface = {
            width: packet.surfaceWidth, height: packet.surfaceHeight,
            pixels: new Uint8ClampedArray(packet.surfaceWidth * packet.surfaceHeight * 4), dirty: null,
          };
        }
        compositeFrameRects(runtime.surface, packet.rects);
        runtime.surface.dirty = null;
        runtime.bytes += bytes.byteLength;
        for (const listener of runtime.listeners) listener(bytes);
      } catch (error) {
        console.error('Failed to consume session frame:', error);
      }
    });
    id = await tauri.connect(connectionId, runtime.channel);
    if (runtime.disposed) {
      await tauri.disconnect(id);
      throw new Error('Session closed while connecting');
    }
    runtimes.set(id, runtime);
    useSessionStore.getState().addSession({
      id, connectionId, connectionName: existing?.connectionName ?? '', host: existing?.host ?? '',
      status: 'connecting', fps: 0, latency: 0, bandwidth: 0,
      width: runtime.surface?.width ?? 0, height: runtime.surface?.height ?? 0,
      reconnectAttempts: 0, maxReconnectAttempts: 5, lastError: null,
    });
    const sessionId = id;
    let eventReceived = false;
    const applyInfo = (info: SessionInfo) => {
      if (runtime.disposed) return;
      const current = useSessionStore.getState().sessions.get(sessionId);
      useSessionStore.getState().updateSession(sessionId, {
        ...info, fps: current?.fps ?? 0, bandwidth: current?.bandwidth ?? 0,
      });
    };
    const unlisten = await listen<SessionInfo>(`session-info-${id}`, (event) => {
      eventReceived = true;
      applyInfo(event.payload);
    });
    if (runtime.disposed) {
      unlisten();
      return id;
    }
    runtime.unlisten = unlisten;
    const info = await tauri.getSessionInfo(id);
    if (info && !eventReceived) applyInfo(info);
    if (runtime.disposed) return id;
    runtime.timer = setInterval(() => {
      useSessionStore.getState().updateSession(sessionId, { fps: runtime.frames, bandwidth: runtime.bytes });
      runtime.frames = 0;
      runtime.bytes = 0;
    }, 1000);
    return id;
  } catch (error) {
    dispose(runtime);
    if (id && runtimes.get(id) === runtime) {
      runtimes.delete(id);
      useSessionStore.getState().removeSession(id);
      await tauri.disconnect(id).catch(() => {});
    } else if (existing && runtimes.get(existing.id) === runtime) {
      runtimes.delete(existing.id);
    }
    throw error;
  }
}
