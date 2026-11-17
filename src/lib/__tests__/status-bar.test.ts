import { describe, expect, it } from 'vitest';

import { buildStatusBarText, sessionCountLabel, statusBarRevealReducer, statusBarRowHeight } from '../status-bar';
import type { StatusBarSession } from '../status-bar';

const session = (overrides: Partial<StatusBarSession>): StatusBarSession => ({
  connectionId: 'conn-1',
  width: 800,
  height: 600,
  status: 'connected',
  ...overrides,
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
