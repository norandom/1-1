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

  it('marks the page for the native layer and drops the mark on a fallback', async () => {
    invoke.mockResolvedValue({ path: 'native', reason: 'native layer available' });
    const { listen } = await import('@tauri-apps/api/event');
    let fallback: ((event: { payload: unknown }) => void) | undefined;
    vi.mocked(listen).mockImplementation(((_: string, handler: (event: { payload: unknown }) => void) => {
      fallback = handler;
      return Promise.resolve(() => {});
    }) as never);
    const { syncDisplayPathClass } = await import('../display-path');
    const classes = new Set<string>();
    const root = {
      classList: {
        toggle: (name: string, on: boolean) => (on ? classes.add(name) : classes.delete(name), on),
        contains: (name: string) => classes.has(name),
      },
    } as unknown as HTMLElement;
    const showBackground = vi.fn();
    syncDisplayPathClass(root, showBackground);
    await vi.waitFor(() => expect(root.classList.contains('native-layer')).toBe(true));
    expect(showBackground).toHaveBeenCalled();
    fallback?.({ payload: { path: 'canvas', reason: 'native layer failed: lost' } });
    expect(root.classList.contains('native-layer')).toBe(false);
  });
});
