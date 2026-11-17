import { beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { DEFAULT_HOST_PROFILE, normalizeHostProfile } from '../host-profile';
import { listConnections, getConnection, updateConnection } from '../tauri';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mock = vi.mocked(invoke);
beforeEach(() => mock.mockReset());
it('defaults missing fields and retains explicit false values', () => {
  expect(normalizeHostProfile()).toEqual(DEFAULT_HOST_PROFILE);
  expect(normalizeHostProfile({ fullWindowDrag: false, cursorBlinking: false }))
    .toEqual({ ...DEFAULT_HOST_PROFILE, fullWindowDrag: false, cursorBlinking: false });
});
it('normalizes legacy and partial profiles at read ingress', async () => {
  mock.mockResolvedValueOnce([{ id: 'legacy' }, { id: 'partial', hostProfile: { fullWindowDrag: false } }]);
  const connections = await listConnections();
  expect(connections[0].hostProfile).toEqual(DEFAULT_HOST_PROFILE);
  expect(connections[1].hostProfile).toEqual({ ...DEFAULT_HOST_PROFILE, fullWindowDrag: false });
  mock.mockResolvedValueOnce({ id: 'legacy' });
  expect((await getConnection('legacy'))?.hostProfile).toEqual(DEFAULT_HOST_PROFILE);
  mock.mockResolvedValueOnce(null);
  expect(await getConnection('missing')).toBeNull();
});
it('keeps omitted profiles omitted in updates while normalizing responses', async () => {
  mock.mockResolvedValueOnce({ id: 'existing', hostProfile: { fullWindowDrag: false } });
  const result = await updateConnection('existing', { name: 'Renamed' });
  expect(mock).toHaveBeenCalledWith('update_connection', { id: 'existing', config: { name: 'Renamed' } });
  expect(result.hostProfile?.fullWindowDrag).toBe(false);
});
