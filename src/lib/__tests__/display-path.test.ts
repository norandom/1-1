import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

describe('display path', () => {
  beforeEach(() => {
    vi.resetModules();
    invoke.mockReset();
  });

  it('asks the backend once and keeps the answer', async () => {
    invoke.mockResolvedValue({ path: 'native', reason: 'native layer available' });
    const { getDisplayPath } = await import('../display-path');
    expect(await getDisplayPath()).toEqual({ path: 'native', reason: 'native layer available' });
    await getDisplayPath();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('get_display_path');
  });

  it('falls back to the canvas when the backend cannot answer', async () => {
    invoke.mockRejectedValue(new Error('no backend'));
    const { getDisplayPath } = await import('../display-path');
    expect((await getDisplayPath()).path).toBe('canvas');
  });
});
