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
  autoHideStatusBar: false,
  showPerformanceHud: false,
  sidebarCollapsed: false,
  sidebarHidden: false,
  sidebarWidth: 260,
  defaultResolution: '1920x1080',
  colorDepth: 32,
  quality: 80,
  reconnectTimeout: 5000,
  connectionTimeout: 30000,
} as const;

describe('defaultSettings', () => {
  it('pins the frontend defaults', () => {
    expect(defaultSettings).toEqual({
      theme: 'system',
      defaultPort: 3389,
      autoReconnect: true,
      autoHideStatusBar: false,
      showPerformanceHud: false,
      sidebarCollapsed: false,
      sidebarHidden: false,
      sidebarWidth: 260,
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
  it('defaults a legacy settings response to a visible status bar', async () => {
    const legacy = { ...defaultSettings };
    Reflect.deleteProperty(legacy, 'autoHideStatusBar');
    vi.mocked(tauri.getSettings).mockResolvedValueOnce(legacy);
    await useSettingsStore.getState().fetchSettings();
    expect(useSettingsStore.getState().settings.autoHideStatusBar).toBe(false);
  });
  it('serializes saves and retains both confirmed patches', async () => {
    let complete!: (value: typeof defaultSettings) => void;
    vi.mocked(tauri.updateSettings).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
    vi.mocked(tauri.updateSettings).mockResolvedValueOnce({ ...defaultSettings, autoHideStatusBar: true, theme: 'dark' });
    const first = useSettingsStore.getState().updateSettings({ autoHideStatusBar: true });
    const second = useSettingsStore.getState().updateSettings({ theme: 'dark' });
    await Promise.resolve();
    expect(tauri.updateSettings).toHaveBeenCalledTimes(1);
    complete({ ...defaultSettings, autoHideStatusBar: true });
    await Promise.all([first, second]);
    expect(useSettingsStore.getState().settings).toMatchObject({ autoHideStatusBar: true, theme: 'dark' });
    expect(useSettingsStore.getState().isSaving).toBe(false);
  });
  it('does not publish auto-hide before the backend confirms persistence', async () => {
    let complete!: (value: typeof defaultSettings) => void;
    vi.mocked(tauri.updateSettings).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
    const saving = useSettingsStore.getState().updateSettings({ autoHideStatusBar: true });
    await Promise.resolve();
    expect(useSettingsStore.getState().settings.autoHideStatusBar).toBe(false);
    complete({ ...defaultSettings, autoHideStatusBar: true });
    await saving;
    expect(useSettingsStore.getState().settings.autoHideStatusBar).toBe(true);
  });
  beforeEach(() => {
    vi.clearAllMocks();
    useSettingsStore.setState({
      settings: { ...defaultSettings },
      isLoading: false,
      isSaving: false,
      error: null,
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

  it('updateSettings publishes confirmed settings and forwards only the partial', async () => {
    vi.mocked(tauri.updateSettings).mockResolvedValueOnce({ ...defaultSettings, theme: 'light' });

    await useSettingsStore.getState().updateSettings({ theme: 'light' });

    expect(useSettingsStore.getState().settings).toEqual({
      ...defaultSettings,
      theme: 'light',
    });
    expect(tauri.updateSettings).toHaveBeenCalledWith({ theme: 'light' });
  });

  it('persists the sidebar rail preference through AppSettings.sidebarCollapsed (task 8.2, Req 10.11)', async () => {
    // RAIL_STATE_PINS: the collapse/expand preference persists via the
    // real setting — tree selection/expansion live in libraryStore's
    // localStorage instead, so this is the only rail persistence path.
    vi.mocked(tauri.updateSettings).mockResolvedValueOnce({ ...defaultSettings, sidebarCollapsed: true });

    await useSettingsStore.getState().updateSettings({ sidebarCollapsed: true });

    expect(useSettingsStore.getState().settings.sidebarCollapsed).toBe(true);
    expect(tauri.updateSettings).toHaveBeenCalledWith({ sidebarCollapsed: true });
  });

  it('retains saved settings and surfaces a failed write', async () => {
    const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.mocked(tauri.updateSettings).mockRejectedValueOnce(new Error('write failed'));

    await useSettingsStore.getState().updateSettings({ quality: 50 });

    expect(useSettingsStore.getState().settings.quality).toBe(80);
    expect(useSettingsStore.getState().error).toContain('write failed');
    expect(consoleSpy).toHaveBeenCalledOnce();
    consoleSpy.mockRestore();
  });
});
