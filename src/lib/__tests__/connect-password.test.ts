import { beforeEach, expect, it, vi } from 'vitest';
import type { ConnectionConfig, HostProfile } from '../../types';

vi.mock('../tauri', () => ({ hasSavedPassword: vi.fn() }));

import * as tauri from '../tauri';
import { CONNECT_CANCELLED, connectPassword } from '../connect-password';
import { DEFAULT_HOST_PROFILE } from '../host-profile';
import { useConnectionStore } from '../../stores/connectionStore';
import { usePasswordPrompt } from '../../stores/passwordPromptStore';

const update = vi.fn(async () => {});

function host(profile: Partial<HostProfile>) {
  const connection = {
    id: 'h1', name: 'Build box', host: 'build.invalid', port: 3389, username: 'dev',
    hostProfile: { ...DEFAULT_HOST_PROFILE, ...profile },
  } as unknown as ConnectionConfig;
  useConnectionStore.setState({ connections: [connection], updateConnection: update });
}

/** Answers the prompt once it opens. */
async function answer(value: { password: string; save: boolean } | null) {
  await vi.waitFor(() => expect(usePasswordPrompt.getState().request).not.toBeNull());
  const request = usePasswordPrompt.getState().request!;
  usePasswordPrompt.getState().answer(value);
  return request;
}

beforeEach(() => {
  vi.mocked(tauri.hasSavedPassword).mockReset();
  update.mockClear();
  usePasswordPrompt.setState({ request: null });
});

it('uses the keychain without asking when a password is saved', async () => {
  host({ savePassword: true });
  vi.mocked(tauri.hasSavedPassword).mockResolvedValue(true);
  await expect(connectPassword('h1')).resolves.toBeUndefined();
  expect(usePasswordPrompt.getState().request).toBeNull();
});

it('asks every time for a host that does not save its password, and never saves it', async () => {
  host({ savePassword: false });
  const asked = connectPassword('h1');
  const request = await answer({ password: 'typed', save: true });
  await expect(asked).resolves.toBe('typed');
  expect(request.reason).toBe('notSaved');
  expect(tauri.hasSavedPassword).not.toHaveBeenCalled();
  expect(update).not.toHaveBeenCalled();
});

it('asks when nothing is saved and saves only on request', async () => {
  host({ savePassword: true });
  vi.mocked(tauri.hasSavedPassword).mockResolvedValue(false);
  const saved = connectPassword('h1');
  expect((await answer({ password: 'kept', save: true })).reason).toBe('missing');
  await expect(saved).resolves.toBe('kept');
  expect(update).toHaveBeenCalledExactlyOnceWith('h1', { password: 'kept' });

  update.mockClear();
  const once = connectPassword('h1');
  await answer({ password: 'once', save: false });
  await expect(once).resolves.toBe('once');
  expect(update).not.toHaveBeenCalled();
});

it('cancelling stops the connection', async () => {
  host({ savePassword: false });
  const asked = connectPassword('h1');
  await answer(null);
  await expect(asked).rejects.toThrow(CONNECT_CANCELLED);
});

it('leaves an unknown connection to the backend without waiting', () => {
  useConnectionStore.setState({ connections: [] });
  expect(connectPassword('missing')).toBeUndefined();
});
