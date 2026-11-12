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

  it('claims "Connected to" while the session is still connecting or failed_defect_b3_12', () => {
    // B3-12: the status bar text ignores the session's lifecycle status —
    // it reports "Connected to …" even for connecting/reconnecting/error
    // sessions. Pinned as a defect; the fix belongs to later task groups.
    expect(buildStatusBarText(session({ status: 'connecting' }))).toBe(
      'Connected to conn-1 - 800x600'
    );
    expect(buildStatusBarText(session({ status: 'reconnecting' }))).toBe(
      'Connected to conn-1 - 800x600'
    );
    expect(buildStatusBarText(session({ status: 'error' }))).toBe(
      'Connected to conn-1 - 800x600'
    );
  });
});

describe('sessionCountLabel', () => {
  it('pluralizes all counts except one', () => {
    expect(sessionCountLabel(0)).toBe('0 active sessions');
    expect(sessionCountLabel(1)).toBe('1 active session');
    expect(sessionCountLabel(3)).toBe('3 active sessions');
  });
});
