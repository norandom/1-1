import { vi, describe, it, expect, beforeEach } from 'vitest';

// Mock the tauri module before importing the store.
vi.mock('../../lib/tauri', () => ({
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
}));

import { useSettingsStore, defaultSettings } from '../settingsStore';
import * as tauri from '../../lib/tauri';

/** Backend default for connection_timeout (store/settings.rs). */
const BACKEND_CONNECTION_TIMEOUT_DEFAULT_MS = 30000;

describe('defaultSettings', () => {
  it('pins the ten frontend defaults', () => {
    expect(defaultSettings).toEqual({
      theme: 'system',
      defaultPort: 3389,
      autoReconnect: true,
      showPerformanceHud: false,
      sidebarCollapsed: false,
      defaultResolution: '1920x1080',
      colorDepth: 32,
      quality: 80,
      reconnectTimeout: 5000,
      connectionTimeout: 10000,
    });
  });

  it('disagrees with the backend connectionTimeout default_defect_d2', () => {
    // D2 (research.md): frontend default 10000 ms vs backend
    // default_connection_timeout() = 30000 ms in src-tauri/src/store/settings.rs.
    // Every other default agrees. Pinned as a defect; task 5.2 fixes parity.
    expect(defaultSettings.connectionTimeout).toBe(10000);
    expect(defaultSettings.connectionTimeout).not.toBe(
      BACKEND_CONNECTION_TIMEOUT_DEFAULT_MS
    );
  });
});

describe('settingsStore', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useSettingsStore.setState({
      settings: { ...defaultSettings },
      isLoading: false,
    });
  });

  it('fetchSettings replaces state from the backend', async () => {
    const backend = { ...defaultSettings, theme: 'dark' as const, quality: 60 };
    vi.mocked(tauri.getSettings).mockResolvedValueOnce(backend);

    await useSettingsStore.getState().fetchSettings();

    expect(useSettingsStore.getState().settings).toEqual(backend);
    expect(useSettingsStore.getState().isLoading).toBe(false);
  });

  it('fetchSettings keeps current settings when the backend fails', async () => {
    vi.mocked(tauri.getSettings).mockRejectedValueOnce(new Error('ipc down'));

    await useSettingsStore.getState().fetchSettings();

    expect(useSettingsStore.getState().settings).toEqual(defaultSettings);
    expect(useSettingsStore.getState().isLoading).toBe(false);
  });

  it('updateSettings merges optimistically and forwards only the partial', async () => {
    vi.mocked(tauri.updateSettings).mockResolvedValueOnce({ ...defaultSettings, theme: 'light' });

    await useSettingsStore.getState().updateSettings({ theme: 'light' });

    expect(useSettingsStore.getState().settings).toEqual({
      ...defaultSettings,
      theme: 'light',
    });
    expect(tauri.updateSettings).toHaveBeenCalledWith({ theme: 'light' });
  });

  it('keeps the optimistic merge when the backend save fails (silent divergence)', async () => {
    const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.mocked(tauri.updateSettings).mockRejectedValueOnce(new Error('write failed'));

    await useSettingsStore.getState().updateSettings({ quality: 50 });

    expect(useSettingsStore.getState().settings.quality).toBe(50);
    expect(consoleSpy).toHaveBeenCalledOnce();
    consoleSpy.mockRestore();
  });
});
