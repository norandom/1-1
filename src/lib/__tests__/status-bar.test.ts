import { afterEach, describe, expect, it, vi } from 'vitest';

import { buildStatusBarText, createStatusBarHoverReveal, sessionCountLabel, statusBarRevealReducer, statusBarRowHeight } from '../status-bar';
import type { StatusBarSession } from '../status-bar';

const session = (overrides: Partial<StatusBarSession>): StatusBarSession => ({
  connectionId: 'conn-1',
  width: 800,
  height: 600,
  status: 'connected',
  ...overrides,
});

afterEach(() => vi.useRealTimers());

it('requires one second of continuous pointer dwell without restarting for duplicate enters', () => {
  vi.useFakeTimers();
  const dispatch = vi.fn();
  const hover = createStatusBarHoverReveal(dispatch);
  hover.enter();
  vi.advanceTimersByTime(999);
  expect(dispatch).not.toHaveBeenCalled();
  hover.enter();
  vi.advanceTimersByTime(1);
  expect(dispatch).toHaveBeenCalledExactlyOnceWith('pointer-enter');
  hover.leave();
  expect(dispatch).toHaveBeenLastCalledWith('pointer-leave');
});

it('cancels pending reveal on leave or lifecycle cleanup and requires a fresh dwell', () => {
  vi.useFakeTimers();
  const dispatch = vi.fn();
  const hover = createStatusBarHoverReveal(dispatch);
  hover.enter();
  vi.advanceTimersByTime(600);
  hover.leave(); // Also the component cleanup on route/mode change and unmount.
  dispatch.mockClear();
  vi.advanceTimersByTime(1000);
  expect(dispatch).not.toHaveBeenCalled();
  hover.enter();
  vi.advanceTimersByTime(999);
  expect(dispatch).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(dispatch).toHaveBeenCalledExactlyOnceWith('pointer-enter');
});

it('keeps keyboard reveal immediate and preserves pointer ownership of an already focused bar', () => {
  vi.useFakeTimers();
  let state = { pointer: false, focus: false };
  const hover = createStatusBarHoverReveal((event) => { state = statusBarRevealReducer(state, event); });
  hover.enter();
  state = statusBarRevealReducer(state, 'focus-enter');
  expect(statusBarRowHeight(true, state)).toBe(28);
  hover.enter(true);
  state = statusBarRevealReducer(state, 'focus-leave');
  expect(statusBarRowHeight(true, state)).toBe(28);
  hover.leave();
  expect(statusBarRowHeight(true, state)).toBe(0);
  vi.advanceTimersByTime(1000);
  expect(statusBarRowHeight(true, state)).toBe(0);
});

it('reserves one row while pointer or focus stays inside and only reclaims it after both leave', () => {
  let state = { pointer: false, focus: false };
  expect(statusBarRowHeight(true, state)).toBe(0);
  state = statusBarRevealReducer(state, 'pointer-enter');
  expect(statusBarRowHeight(true, state)).toBe(28);
  expect(statusBarRevealReducer(state, 'pointer-enter')).toBe(state);
  state = statusBarRevealReducer(state, 'focus-enter');
  state = statusBarRevealReducer(state, 'pointer-leave');
  expect(statusBarRowHeight(true, state)).toBe(28);
  state = statusBarRevealReducer(state, 'focus-leave');
  expect(statusBarRowHeight(true, state)).toBe(0);
  expect(statusBarRowHeight(false, state)).toBe(28);
});

describe('buildStatusBarText', () => {
  it.each(['connecting', 'connected', 'reconnecting', 'error', 'disconnected'] as const)('resolves readable host labels for %s without leaking IDs', (status) => {
    const current = session({ connectionId: '44444444-secret-id', status });
    expect(buildStatusBarText(current, { name: ' Production ', host: 'server.example' })).toContain('Production');
    expect(buildStatusBarText(current, { name: ' ', host: 'server.example' })).toContain('server.example');
    expect(buildStatusBarText(current)).toContain('Unknown host');
    expect(buildStatusBarText(current)).not.toContain(current.connectionId);
  });
  it('shows "No active connection" without a session', () => {
    expect(buildStatusBarText(null)).toBe('No active connection');
  });

  it('names the connection and its resolution for an active session', () => {
    expect(buildStatusBarText(session({ connectionId: 'c9', width: 1920, height: 1080 }), { name: 'Build server', host: 'build.example' })).toBe(
      'Connected to Build server - 1920x1080'
    );
  });

  it.each([
    ['connecting', 'Connecting to Unknown host'],
    ['reconnecting', 'Reconnecting to Unknown host'],
    ['error', 'Connection error - Unknown host'],
    ['disconnected', 'Disconnected - Unknown host'],
  ] as const)('describes %s without claiming a connected desktop', (status, text) => {
    expect(buildStatusBarText(session({ status }))).toBe(text);
  });
});

describe('sessionCountLabel', () => {
  it('pluralizes all counts except one', () => {
    expect(sessionCountLabel(0)).toBe('0 active sessions');
    expect(sessionCountLabel(1)).toBe('1 active session');
    expect(sessionCountLabel(3)).toBe('3 active sessions');
  });
});
