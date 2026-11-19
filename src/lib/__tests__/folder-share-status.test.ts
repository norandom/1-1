import { describe, expect, it } from 'vitest';
import type { FolderShareStatus, SessionInfoWire } from '../../types';
import { normalizeSessionInfo } from '../session-info';
import { shareStatusDetail, shareStatusLabel, sharesSummary } from '../folder-share';

/** Session events as the backend sends them, with the share's JSON as in its Rust test. */
function event(folderShare: FolderShareStatus): SessionInfoWire {
  return {
    id: 's1', connectionId: 'h1', connectionName: 'Host', host: 'host.test', status: 'connected',
    fps: 0, latency: 0, bandwidth: 0, width: 1280, height: 720, connectedAt: null,
    reconnectAttempts: 0, maxReconnectAttempts: 5, lastError: null, folderShares: [folderShare],
  } as SessionInfoWire;
}

describe('folder share status in the session details', () => {
  it('follows the protocol evidence from setup to use', () => {
    const drive = { name: 'Work', readOnly: false, lastFailure: null };
    const seen = (['negotiating', 'offered', 'accepted', 'accessObserved', 'ended'] as const)
      .map((state) => normalizeSessionInfo(event({ state, ...drive })).folderShares![0])
      .map(shareStatusLabel);
    expect(seen).toEqual(['connecting', 'offered', 'accepted', 'in use', 'ended']);
  });

  it('keeps a failure beside the state, so it never reads as access', () => {
    const share = normalizeSessionInfo(event({
      state: 'accepted', name: 'Work', readOnly: true,
      lastFailure: { operation: 'Write', status: '0xC00000A2', description: 'The share is read-only' },
    })).folderShares![0];
    expect(shareStatusLabel(share)).toBe('accepted');
    expect(shareStatusDetail(share)).toMatch(/No file on it has been used yet/);
    expect(share.lastFailure?.description).toBe('The share is read-only');
  });

  it('keeps each reason for no share distinct', () => {
    const outcomes: FolderShareStatus[] = [
      { state: 'unverified', reason: "The server's identity changed since the folder was chosen." },
      { state: 'folderUnavailable', reason: 'The shared folder is not available.' },
      { state: 'unknown', reason: 'The shared folder could not be opened.' },
      { state: 'unsupported', reason: 'Folder sharing is available only in the Windows build.' },
    ];
    expect(outcomes.map(shareStatusDetail)).toEqual(outcomes.map((o) => 'reason' in o ? o.reason : ''));
    expect(outcomes.map(shareStatusLabel)).toEqual(['not shared', 'not shared', 'not shared', 'not available']);
    const refused = shareStatusDetail({ state: 'refused', status: '0xC0000001' });
    expect(refused).toContain('0xC0000001');
    expect(refused).toMatch(/does not say why/);
  });

  it('shows nothing for a host without a folder', () => {
    expect(normalizeSessionInfo(event({ state: 'off' })).folderShares?.[0]?.state).toBe('off');
  });
});

describe('several shared folders', () => {
  it('summarize by the furthest folder and count them', () => {
    expect(sharesSummary([{ state: 'accessObserved' }])).toBe('Folder: in use');
    expect(sharesSummary([
      { state: 'folderUnavailable', reason: 'gone' }, { state: 'accepted' }, { state: 'accessObserved' },
    ])).toBe('Folders (3): in use');
    expect(sharesSummary([{ state: 'refused', status: '0xC0000001' }, { state: 'unverified', reason: 'key' }])).toBe('Folders (2): refused');
    expect(sharesSummary([])).toBe('');
  });
});

describe('several shared folders', () => {
  it('summarize by the furthest folder and count them', () => {
    expect(sharesSummary([{ state: 'accessObserved' }])).toBe('Folder: in use');
    expect(sharesSummary([
      { state: 'folderUnavailable', reason: 'gone' }, { state: 'accepted' }, { state: 'accessObserved' },
    ])).toBe('Folders (3): in use');
    expect(sharesSummary([{ state: 'refused', status: '0xC0000001' }, { state: 'unverified', reason: 'key' }])).toBe('Folders (2): refused');
    expect(sharesSummary([])).toBe('');
  });
});
