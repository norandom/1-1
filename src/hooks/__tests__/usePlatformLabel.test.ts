import { vi, describe, it, expect, beforeEach } from 'vitest';
import { createElement } from 'react';
import { renderToString } from 'react-dom/server';

// The Tauri OS plugin is unavailable outside the app (it would throw); the
// hook must fall back to the user agent, so every test drives the mock.
vi.mock('@tauri-apps/plugin-os', () => ({
  platform: vi.fn(),
}));

import { platform } from '@tauri-apps/plugin-os';
import {
  detectPlatform,
  normalizePlatform,
  usePlatformLabel,
  type PlatformLabel,
} from '../usePlatformLabel';

const platformMock = vi.mocked(platform);

const WINDOWS_UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)';
const MAC_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)';
const LINUX_UA = 'Mozilla/5.0 (X11; Linux x86_64)';

beforeEach(() => {
  platformMock.mockReset();
});

describe('normalizePlatform', () => {
  it('keeps the three desktop platforms and maps everything else to unknown', () => {
    expect(normalizePlatform('macos')).toBe('macos');
    expect(normalizePlatform('windows')).toBe('windows');
    expect(normalizePlatform('linux')).toBe('linux');
    expect(normalizePlatform('android')).toBe('unknown');
    expect(normalizePlatform('ios')).toBe('unknown');
    expect(normalizePlatform(undefined)).toBe('unknown');
  });
});

describe('detectPlatform', () => {
  it('prefers the Tauri OS plugin over the user agent', () => {
    platformMock.mockReturnValue('windows');
    expect(detectPlatform(MAC_UA)).toBe('windows');
    platformMock.mockReturnValue('macos');
    expect(detectPlatform(WINDOWS_UA)).toBe('macos');
  });

  it('falls back to the user agent when the plugin is unavailable', () => {
    platformMock.mockImplementation(() => {
      throw new Error('not inside Tauri');
    });
    expect(detectPlatform(WINDOWS_UA)).toBe('windows');
    expect(detectPlatform(MAC_UA)).toBe('macos');
    expect(detectPlatform(LINUX_UA)).toBe('linux');
    expect(detectPlatform('Mozilla/5.0 (compatible)').length).toBeGreaterThan(0);
  });

  it('falls back to the user agent when the plugin reports a non-desktop OS', () => {
    platformMock.mockReturnValue('freebsd');
    expect(detectPlatform(WINDOWS_UA)).toBe('windows');
  });
});

describe('usePlatformLabel', () => {
  const renderProbe = (): PlatformLabel => {
    function Probe() {
      // Render the hook result as text (pure render, no side effects);
      // renderToString HTML-escapes it, so unescape before parsing.
      return JSON.stringify(usePlatformLabel());
    }
    const html = renderToString(createElement(Probe));
    const json = html
      .replace(/&quot;/g, '"')
      .replace(/&lt;/g, '<')
      .replace(/&gt;/g, '>')
      .replace(/&amp;/g, '&');
    return JSON.parse(json) as PlatformLabel;
  };

  it('labels Windows with Ctrl (Req 8.5)', () => {
    platformMock.mockReturnValue('windows');
    expect(renderProbe()).toEqual({
      platform: 'windows',
      isMac: false,
      modLabel: 'Ctrl',
    });
  });

  it('labels Linux with Ctrl (Req 8.5)', () => {
    platformMock.mockReturnValue('linux');
    expect(renderProbe()).toEqual({
      platform: 'linux',
      isMac: false,
      modLabel: 'Ctrl',
    });
  });

  it('labels macOS with ⌘', () => {
    platformMock.mockReturnValue('macos');
    expect(renderProbe()).toEqual({
      platform: 'macos',
      isMac: true,
      modLabel: '\u2318',
    });
  });

  it('defaults to Ctrl hints when no platform can be detected', () => {
    platformMock.mockImplementation(() => {
      throw new Error('not inside Tauri');
    });
    expect(renderProbe()?.modLabel).toBe('Ctrl');
  });
});
