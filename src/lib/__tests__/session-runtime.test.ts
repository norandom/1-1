import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FrameMessage } from '../frame-protocol';
import type { SessionInfo } from '../../types';

const transport = vi.hoisted(() => ({
  channels: new Map<string, (frame: FrameMessage) => void>(),
  statuses: new Map<string, (event: { payload: SessionInfo }) => void>(),
  unlisten: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {
    onmessage: (frame: FrameMessage) => void;
    constructor(listener: (frame: FrameMessage) => void) { this.onmessage = listener; }
  },
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name, listener) => {
    transport.statuses.set(name, listener);
    return transport.unlisten;
  }),
}));
vi.mock('../tauri', () => ({
  connect: vi.fn(async (id, channel) => {
    transport.channels.set(id, channel.onmessage);
    return `session-${id}`;
  }),
  getSessionInfo: vi.fn(async () => null),
  disconnect: vi.fn(async () => {}),
  frameConsumed: vi.fn(async () => {}),
}));

import * as tauri from '../tauri';
import { listen } from '@tauri-apps/api/event';
import { useSessionStore } from '../../stores/sessionStore';
import { connectSession, subscribeSessionFrames, refreshSessionDisplay } from '../session-runtime';
import { parseFramePacket } from '../frame-protocol';

// Real wire layout, small surfaces: every pixel channel is distinct across sessions.
function packet(value: number, x = 0, width = 2, surfaceWidth = 2): Uint8Array {
  const bytes = new Uint8Array(15 + width * 4);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, width === surfaceWidth ? 1 : 2);
  view.setUint16(1, surfaceWidth, true);
  view.setUint16(3, 1, true);
  view.setUint16(5, 1, true);
  view.setUint16(7, x, true);
  view.setUint16(11, width, true);
  view.setUint16(13, 1, true);
  bytes.fill(value, 15);
  return bytes;
}

let sequence = 0;
function send(connection: string, frame: Uint8Array) {
  transport.channels.get(connection)!({ kind: 'frameDelivery', sessionId: 'session-' + connection, generation: 'test', sequence: ++sequence });
  transport.channels.get(connection)!(frame);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  transport.channels.clear();
  transport.statuses.clear();
  useSessionStore.setState({ sessions: new Map(), activeSessionId: null });
});
afterEach(() => {
  for (const id of useSessionStore.getState().sessions.keys()) useSessionStore.getState().removeSession(id);
  vi.useRealTimers();
});

describe('session-owned frame channels (8.6 integration remediation)', () => {
  it('rejects binary without metadata, malformed metadata, and another session receipt', async () => {
    const id = await connectSession('ack');
    const listener = vi.fn();
    subscribeSessionFrames(id, listener);
    transport.channels.get('ack')!(packet(5));
    transport.channels.get('ack')!({ kind: 'frameDelivery', sessionId: 'wrong', generation: 'g', sequence: 1 });
    transport.channels.get('ack')!(packet(5));
    transport.channels.get('ack')!({ kind: 'frameDelivery', sessionId: id, generation: 'g', sequence: -1 });
    transport.channels.get('ack')!(packet(5));
    expect(listener).not.toHaveBeenCalled();
    expect(tauri.frameConsumed).not.toHaveBeenCalled();
  });
  it('can resend the last exact receipt for explicit full-refresh recovery', async () => {
    const id = await connectSession('ack');
    transport.channels.get('ack')!({ kind: 'frameDelivery', sessionId: id, generation: 'gen', sequence: 1 });
    transport.channels.get('ack')!(packet(5));
    refreshSessionDisplay(id);
    expect(tauri.frameConsumed).toHaveBeenLastCalledWith(id, { generation: 'gen', sequence: 1 }, true);
  });
  it('acknowledges the exact receipt only after active canvas completion', async () => {
    const id = await connectSession('ack');
    const listener = vi.fn();
    subscribeSessionFrames(id, listener);
    transport.channels.get('ack')!({ kind: 'frameDelivery', sessionId: id, generation: 'gen', sequence: 1 });
    transport.channels.get('ack')!(packet(5));
    expect(tauri.frameConsumed).not.toHaveBeenCalled();
    const complete = listener.mock.calls[0][1];
    expect(complete).toBeTypeOf('function');
    complete();
    complete();
    expect(tauri.frameConsumed).toHaveBeenCalledExactlyOnceWith(id, { generation: 'gen', sequence: 1 }, false);
  });
  it('consumes background frames into the retained image and releases on unmount', async () => {
    const id = await connectSession('ack');
    const listener = vi.fn();
    const detach = subscribeSessionFrames(id, listener);
    transport.channels.get('ack')!({ kind: 'frameDelivery', sessionId: id, generation: 'gen', sequence: 1 });
    transport.channels.get('ack')!(packet(5));
    detach();
    expect(tauri.frameConsumed).toHaveBeenCalledTimes(1);
    transport.channels.get('ack')!({ kind: 'frameDelivery', sessionId: id, generation: 'gen', sequence: 2 });
    transport.channels.get('ack')!(packet(9));
    expect(tauri.frameConsumed).toHaveBeenCalledTimes(2);
    listener.mock.calls[0][1](); // obsolete canvas completion must not ACK new flight
    expect(tauri.frameConsumed).toHaveBeenCalledTimes(2);
  });
  it('switches A/B without routing background frames into the visible session', async () => {
    const a = await connectSession('a');
    const b = await connectSession('b');
    const framesA = vi.fn();
    const framesB = vi.fn();
    const detachA = subscribeSessionFrames(a, framesA);
    send('a', packet(11));
    detachA();
    const detachB = subscribeSessionFrames(b, framesB);
    send('a', packet(22));
    send('b', packet(99));
    expect(framesA).toHaveBeenCalledTimes(1);
    expect(framesB).toHaveBeenCalledTimes(1);
    expect(parseFramePacket(framesB.mock.calls[0][0]).rects[0].pixels[0]).toBe(99);
    detachB();
  });

  it('returns from a dashboard with the complete latest image and no connect IPC', async () => {
    const a = await connectSession('a');
    send('a', packet(11));
    const detach = subscribeSessionFrames(a, vi.fn());
    detach(); // view unmount: background dirty regions must still compose
    send('a', packet(22, 1, 1));
    const returned = vi.fn();
    subscribeSessionFrames(a, returned);
    expect(await connectSession('a')).toBe(a);
    expect(tauri.connect).toHaveBeenCalledTimes(1);
    const image = parseFramePacket(returned.mock.calls[0][0]);
    expect(image.kind).toBe('full-frame');
    expect([...image.rects[0].pixels]).toEqual([11,11,11,11,22,22,22,22]);
  });

  it('keeps background status live and disposes resources when the tab is removed', async () => {
    const a = await connectSession('a');
    const info = { ...useSessionStore.getState().sessions.get(a)!, status: 'connected' as const, latency: 17 };
    transport.statuses.get(`session-info-${a}`)!({ payload: info });
    expect(useSessionStore.getState().sessions.get(a)?.latency).toBe(17);
    expect(vi.getTimerCount()).toBe(1);
    useSessionStore.getState().removeSession(a);
    expect(transport.unlisten).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
    transport.statuses.get(`session-info-${a}`)!({ payload: info });
    const afterClose = vi.fn();
    subscribeSessionFrames(a, afterClose);
    send('a', packet(33));
    expect(afterClose).not.toHaveBeenCalled();
    expect(useSessionStore.getState().sessions.has(a)).toBe(false);
  });

  it('only explicitly restarts terminal sessions, keeping identity and rejecting old channel frames', async () => {
    const a = await connectSession('a');
    const oldChannel = transport.channels.get('a')!;
    useSessionStore.getState().updateSession(a, { status: 'disconnected' });
    expect(await connectSession('a')).toBe(a);
    expect(tauri.connect).toHaveBeenCalledTimes(1);
    expect(await connectSession('a', true)).toBe(a);
    expect(tauri.connect).toHaveBeenCalledTimes(2);
    const frames = vi.fn();
    subscribeSessionFrames(a, frames);
    oldChannel(packet(1));
    send('a', packet(2));
    expect(frames).toHaveBeenCalledTimes(1);
    expect(parseFramePacket(frames.mock.calls[0][0]).rects[0].pixels[0]).toBe(2);
  });

  it('coalesces simultaneous connect attempts for the same connection', async () => {
    expect(await Promise.all([connectSession('a'), connectSession('a')])).toEqual(['session-a', 'session-a']);
    expect(tauri.connect).toHaveBeenCalledTimes(1);
  });

  it('replays a resized surface without pixels from the previous dimensions', async () => {
    const a = await connectSession('a');
    send('a', packet(11));
    send('a', packet(33, 0, 1, 1));
    const frames = vi.fn();
    subscribeSessionFrames(a, frames);
    const image = parseFramePacket(frames.mock.calls[0][0]);
    expect(image.surfaceWidth).toBe(1);
    expect([...image.rects[0].pixels]).toEqual([33,33,33,33]);
  });

  it('keeps an attached canvas subscribed through explicit restart and detaches cleanly', async () => {
    const a = await connectSession('a');
    const frames = vi.fn();
    const detach = subscribeSessionFrames(a, frames);
    useSessionStore.getState().updateSession(a, { status: 'error' });
    await connectSession('a', true);
    send('a', packet(42));
    expect(frames).toHaveBeenCalledTimes(1);
    detach();
    send('a', packet(43));
    expect(frames).toHaveBeenCalledTimes(1);
  });

  it('does not resurrect a tab closed while the status listener is registering', async () => {
    let finishListen!: (unlisten: () => void) => void;
    vi.mocked(listen).mockImplementationOnce(() => new Promise((resolve) => { finishListen = resolve; }));
    const connecting = connectSession('a');
    await Promise.resolve();
    expect(useSessionStore.getState().sessions.has('session-a')).toBe(true);
    useSessionStore.getState().removeSession('session-a');
    const unlisten = vi.fn();
    finishListen(unlisten);
    await connecting;
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(useSessionStore.getState().sessions.has('session-a')).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('closes a restarted backend actor when its tab was removed during connect IPC', async () => {
    const a = await connectSession('a');
    useSessionStore.getState().updateSession(a, { status: 'error' });
    let finishConnect!: (id: string) => void;
    vi.mocked(tauri.connect).mockImplementationOnce(() => new Promise((resolve) => { finishConnect = resolve; }));
    const restarting = connectSession('a', true);
    useSessionStore.getState().removeSession(a);
    finishConnect(a);
    await expect(restarting).rejects.toThrow('Session closed while connecting');
    expect(tauri.disconnect).toHaveBeenCalledExactlyOnceWith(a);
    expect(useSessionStore.getState().sessions.has(a)).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('can retry a failed initial connect without retaining a dead runtime', async () => {
    vi.mocked(tauri.connect).mockRejectedValueOnce(new Error('refused'));
    await expect(connectSession('a')).rejects.toThrow('refused');
    expect(await connectSession('a')).toBe('session-a');
    expect(tauri.connect).toHaveBeenCalledTimes(2);
  });
});
