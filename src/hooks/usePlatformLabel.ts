import { useState } from 'react';
import { platform } from '@tauri-apps/plugin-os';
import {
  modLabelForPlatform,
  type DesktopPlatform,
  type ModLabel,
} from '../lib/shortcuts';

export interface PlatformLabel {
  platform: DesktopPlatform;
  isMac: boolean;
  /** ⌘ on macOS, Ctrl on Windows/Linux — shortcut hints resolve via this (Req 8.5). */
  modLabel: ModLabel;
}

/** Narrow a @tauri-apps/plugin-os platform string to the desktop set. */
export function normalizePlatform(os: string | undefined): DesktopPlatform {
  return os === 'macos' || os === 'windows' || os === 'linux' ? os : 'unknown';
}

/**
 * Platform detection with the fallback chain the Titlebar established:
 * the Tauri OS plugin first, then the user agent outside Tauri (unit
 * tests, plain web preview).
 */
export function detectPlatform(userAgent: string = navigator.userAgent): DesktopPlatform {
  let os: string | undefined;
  try {
    os = platform();
  } catch {
    // Not inside Tauri: fall through to the user agent.
  }
  const known = normalizePlatform(os);
  if (known !== 'unknown') return known;
  if (userAgent.includes('Mac')) return 'macos';
  if (userAgent.includes('Windows')) return 'windows';
  if (userAgent.includes('Linux')) return 'linux';
  return 'unknown';
}

/**
 * The platform helper for platform-aware UI labels (design.md
 * traceability rows 5 and 8). Reuse this hook instead of adding new
 * detection mechanisms; shortcut hints resolve Ctrl vs ⌘ through
 * `modLabel` (Req 8.5).
 */
export function usePlatformLabel(): PlatformLabel {
  // Lazy initializer: detection is synchronous (plugin-os `platform()` or
  // the user agent), so the label is correct on first paint — no ⌘ flash
  // on Windows and no post-mount probe to re-assert.
  const [detected] = useState<DesktopPlatform>(() => detectPlatform());
  return {
    platform: detected,
    isMac: detected === 'macos',
    modLabel: modLabelForPlatform(detected),
  };
}
