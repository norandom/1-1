import { create } from 'zustand';
import type { AppSettings } from '../types';
import * as tauri from '../lib/tauri';

interface SettingsState {
  settings: AppSettings;
  isLoading: boolean;
  isSaving: boolean;
  error: string | null;
  fetchSettings: () => Promise<void>;
  updateSettings: (settings: Partial<AppSettings>) => Promise<void>;
}

export const defaultSettings: AppSettings = {
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
  // Req 5.2 (D2 fixed, task 5.2): matches backend
  // default_connection_timeout() in src-tauri/src/store/settings.rs;
  // parity pinned in settingsStore.test.ts (BACKEND_DEFAULTS).
  connectionTimeout: 30000,
};

export const useSettingsStore = create<SettingsState>((set) => {
  let pending = Promise.resolve();
  return {
    settings: defaultSettings,
    isLoading: false,
    isSaving: false,
    error: null,

    fetchSettings: async () => {
      set({ isLoading: true });
      try {
        const settings = await tauri.getSettings();
        set({ settings: { ...defaultSettings, ...settings }, isLoading: false });
      } catch {
        set({ isLoading: false });
      }
    },

    updateSettings: async (partial) => {
      set({ isSaving: true, error: null });
      const saving = pending.then(async () => {
        try {
          const settings = await tauri.updateSettings(partial);
          set({ settings: { ...defaultSettings, ...settings }, error: null });
        } catch (e) {
          set({ error: `Settings not saved: ${e instanceof Error ? e.message : String(e)}` });
          console.error('Failed to save settings:', e);
        }
      });
      pending = saving;
      await saving;
      if (pending === saving) set({ isSaving: false });
    },
  };
});
