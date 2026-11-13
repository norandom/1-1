import { vi, describe, it, expect, beforeEach } from 'vitest';

// Mock the tauri module before importing the store.
vi.mock('../../lib/tauri', () => ({
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
}));

import { useSettingsStore, defaultSettings } from '../settingsStore';
import * as tauri from '../../lib/tauri';

/**
 * Backend serde defaults (src-tauri/src/store/settings.rs `default_*` fns,
 * pinned by `test_settings_defaults_current_values`). Req 5.2: frontend and
 * backend defaults shall agree for every setting — both literals move
 * together; there is deliberately no code generation between them.
 */
const BACKEND_DEFAULTS = {
  theme: 'system',
  defaultPort: 3389,
  autoReconnect: true,
  showPerformanceHud: false,
  sidebarCollapsed: false,
  defaultResolution: '1920x1080',
  colorDepth: 32,
  quality: 80,
  reconnectTimeout: 5000,
  connectionTimeout: 30000,
} as const;

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
      connectionTimeout: 30000,
    });
  });

  it('agrees with the backend serde defaults for every setting (D2 fixed)', () => {
    // D2 (research.md): connectionTimeout used to diverge (TS 10000 vs Rust
    // default_connection_timeout() = 30000). Task 5.2 restored parity; this
    // is the Req 5.2 default-parity pin mirroring the Rust-side literal.
    expect(defaultSettings).toEqual(BACKEND_DEFAULTS);
    expect(defaultSettings.connectionTimeout).toBe(BACKEND_DEFAULTS.connectionTimeout);
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
