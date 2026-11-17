import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, expect, it } from 'vitest';
import { StatusBar } from './StatusBar';
import { useConnectionStore } from '../../stores/connectionStore';
import { defaultSettings, useSettingsStore } from '../../stores/settingsStore';
import { useSessionStore } from '../../stores/sessionStore';

const original = useSettingsStore.getInitialState().settings;
afterEach(() => {
  useSettingsStore.getInitialState().settings = original;
  useConnectionStore.getInitialState().connections = [];
  useSessionStore.getInitialState().sessions = new Map();
  useSessionStore.getInitialState().activeSessionId = null;
});

it('reclaims the row for valid pending and existing sessions but keeps normal chrome outside them', () => {
  useConnectionStore.getInitialState().connections = [{ id: 'opaque-id', name: 'Build host', host: 'build.example',
    port: 3389, username: '', domain: '', colorAccent: null, tags: [], displayWidth: null,
    displayHeight: null, fontSmoothing: true, createdAt: '', updatedAt: '' }];
  useSettingsStore.getInitialState().settings = { ...defaultSettings, autoHideStatusBar: true };
  const render = (path: string) => renderToStaticMarkup(createElement(MemoryRouter, { initialEntries: [path] }, createElement(StatusBar)));
  const pending = render('/session/opaque-id');
  expect(pending).toContain('data-status-bar-mode="auto-hide"');
  expect(pending).toContain('fixed inset-x-0 bottom-0');
  expect(pending).toContain('aria-label="Show status bar"');
  expect(pending).toContain('data-local-input="true"');
  expect(pending).toContain('style="height:0"');
  expect(pending).toContain('aria-keyshortcuts="Control+Alt+Shift+S"');
  // Only the edge target is fixed; the row itself participates in layout.
  expect(pending).not.toContain('absolute inset-x-0');
  useSessionStore.getInitialState().sessions = new Map([['session-id', { id: 'session-id', connectionId: 'opaque-id',
    connectionName: '', host: '', status: 'connected', fps: 0, latency: 0, bandwidth: 0, width: 940, height: 736,
    reconnectAttempts: 0, maxReconnectAttempts: 5 }]]);
  useSessionStore.getInitialState().activeSessionId = 'session-id';
  expect(render('/session/session-id')).toContain('Connected to Build host - 940x736');
  for (const path of ['/', '/settings', '/session/stale']) {
    expect(render(path)).toContain('data-status-bar-mode="visible"');
    expect(render(path)).not.toContain('Show status bar');
  }
  useSettingsStore.getInitialState().settings = { ...defaultSettings, autoHideStatusBar: false };
  expect(render('/session/session-id')).toContain('data-status-bar-mode="visible"');
  expect(render('/session/session-id')).toContain('style="height:28px"');
});
