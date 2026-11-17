import { describe, expect, it } from 'vitest';

import { buildStatusBarText, sessionCountLabel } from '../status-bar';
import type { StatusBarSession } from '../status-bar';

const session = (overrides: Partial<StatusBarSession>): StatusBarSession => ({
  connectionId: 'conn-1',
  width: 800,
  height: 600,
  status: 'connected',
  ...overrides,
});

describe('buildStatusBarText', () => {
  it('shows "No active connection" without a session', () => {
    expect(buildStatusBarText(null)).toBe('No active connection');
  });

  it('names the connection and its resolution for an active session', () => {
    expect(buildStatusBarText(session({ connectionId: 'c9', width: 1920, height: 1080 }))).toBe(
      'Connected to c9 - 1920x1080'
    );
  });

  it.each([
    ['connecting', 'Connecting to conn-1'],
    ['reconnecting', 'Reconnecting to conn-1'],
    ['error', 'Connection error - conn-1'],
    ['disconnected', 'Disconnected - conn-1'],
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
