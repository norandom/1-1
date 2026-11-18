import { beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import type { CheckedCredential } from '../../types';
import { deleteConnection, deleteLibrary, reloadLibraryStore, testSavedCredentials, updateConnection } from '../../lib/tauri';
import { invalidateLibraryCredentialChecks, runCredentialCheck, useCredentialChecks } from '../credentialCheckStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const call = vi.mocked(invoke);
const current = (id: string) => useCredentialChecks.getState().get(id);
const start = (id: string, library = 'work') => runCredentialCheck(id, library, testSavedCredentials);
const accepted = (connectionId: string): CheckedCredential => ({ connectionId, revision: 'r1', outcome: 'accepted' });

function pendingResult() {
  let finish!: (value: CheckedCredential | null) => void;
  call.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  return (value: CheckedCredential | null) => finish(value);
}

beforeEach(() => { call.mockReset(); invalidateLibraryCredentialChecks(); });

it('checks only a selected ID, coalesces repeat activation, and attributes independent results by host', async () => {
  const finish = pendingResult();
  const first = start('a');
  await start('a');
  expect(current('a')?.outcome).toBe('checking');
  expect(current('b')).toBeUndefined();
  call.mockResolvedValueOnce({ connectionId: 'b', revision: 'b1', outcome: 'rejected' });
  await start('b');
  expect(call.mock.calls).toEqual([
    ['test_saved_credentials', { connectionId: 'a' }], ['test_saved_credentials', { connectionId: 'b' }],
  ]);
  finish(accepted('a'));
  await first;
  expect(current('a')).toMatchObject(accepted('a'));
  expect(current('b')?.outcome).toBe('rejected');
});

it.each(['edit', 'delete', 'cascade', 'reload'])('discards a result across %s and permits an explicit new check', async action => {
  const finish = pendingResult();
  const first = start('a');
  // Failure may follow a partial commit. A prior check cannot establish the effective saved credential.
  call.mockRejectedValueOnce(new Error('synthetic save failure'));
  const mutation = action === 'edit' ? updateConnection('a', { username: 'changed' })
    : action === 'delete' ? deleteConnection('a') : action === 'cascade' ? deleteLibrary('work') : reloadLibraryStore();
  await expect(mutation).rejects.toThrow();
  expect(current('a')).toBeUndefined();
  call.mockResolvedValueOnce({ connectionId: 'a', revision: 'r2', outcome: 'unableToVerify', reason: 'timeout' });
  await start('a');
  finish(accepted('a'));
  await first;
  expect(current('a')).toMatchObject({ revision: 'r2', outcome: 'unableToVerify', reason: 'timeout' });
});

it('discards superseded or mismatched native replies and redacts IPC errors', async () => {
  call.mockResolvedValueOnce(null).mockResolvedValueOnce(accepted('b')).mockRejectedValueOnce('private diagnostic');
  await start('a');
  expect(current('a')).toBeUndefined();
  await start('a');
  expect(current('a')).toBeUndefined();
  await start('a');
  expect(current('a')).toEqual({ libraryId: 'work', outcome: 'unavailable' });
});

it('drops checks racing a library replacement even when the IPC reply arrives after publication', async () => {
  let publish!: () => void;
  call.mockReturnValueOnce(new Promise(resolve => { publish = () => resolve({ mode: 'ready' }); }));
  const reload = reloadLibraryStore();
  const finish = pendingResult();
  const check = start('a');
  publish();
  await reload;
  finish(accepted('a'));
  await check;
  expect(current('a')).toBeUndefined();
});
