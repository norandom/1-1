import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, expect, it } from 'vitest';
import { StatusBar } from './StatusBar';
import { useConnectionStore } from '../../stores/connectionStore';
import { useSettingsStore } from '../../stores/settingsStore';
import { useSessionChrome } from '../../stores/sessionChromeStore';
import { DEFAULT_HOST_PROFILE } from '../../lib/host-profile';
import { useSessionStore } from '../../stores/sessionStore';
import type { SessionInfo } from '../../types';

const original = useSettingsStore.getInitialState().settings;
afterEach(() => {
  useSettingsStore.getInitialState().settings = original;
  useConnectionStore.getInitialState().connections = [];
  useSessionStore.getInitialState().sessions = new Map();
  useSessionStore.getInitialState().activeSessionId = null;
  useSessionChrome.getInitialState().visible = false;
});

it('shows the selected desktop’s confirmed control, image age and unavailable reason inside the status row', () => {
  const first: SessionInfo = {
    id: 'one', connectionId: 'host-one', connectionName: 'Host one', host: 'one.invalid', status: 'connected',
    fps: 0, latency: 0, bandwidth: 0, width: 640, height: 480, reconnectAttempts: 0, maxReconnectAttempts: 5,
    displayPauseAvailable: true, displayPaused: true, idlePreviewState: 'off', idlePreviewUpdatedAt: null,
  };
  const second = { ...first, id: 'two', displayPaused: false, displayPauseAvailable: false,
    displayPauseUnavailableReason: 'Server declined display suppression.' };
  useSessionStore.getInitialState().sessions = new Map([['one', first], ['two', second]]);
  useSessionStore.getInitialState().activeSessionId = 'two';
  const render = (path: string) => renderToStaticMarkup(createElement(MemoryRouter, { initialEntries: [path] }, createElement(StatusBar)));
  expect(render('/session/one')).toContain('Resume live');
  expect(render('/session/one')).toContain('Last update unknown');
  expect(render('/session/two')).toContain('aria-disabled="true"');
  expect(render('/session/two')).toContain('Server declined display suppression.');
  expect(render('/session/two')).not.toContain('Resume live');
  expect(render('/')).not.toContain('Pause display');
  first.displayPaused = false;
  expect(render('/session/one')).toContain('Pause display');
  expect(render('/session/one')).not.toContain('Last update unknown');
});

it('shows the bar with the top toolbar for a host that chose it, and as a row everywhere else', () => {
  const build = { id: 'opaque-id', name: 'Build host', host: 'build.example',
    port: 3389, username: '', domain: '', colorAccent: null, tags: [], displayWidth: null,
    displayHeight: null, fontSmoothing: true, createdAt: '', updatedAt: '' };
  useConnectionStore.getInitialState().connections = [{ ...build, hostProfile: { ...DEFAULT_HOST_PROFILE, bottomBar: 'withToolbar' } }];
  const render = (path: string) => renderToStaticMarkup(createElement(MemoryRouter, { initialEntries: [path] }, createElement(StatusBar)));
  const pending = render('/session/opaque-id');
  expect(pending).toContain('data-status-bar-mode="with-toolbar"');
  // An overlay line: it never takes a layout row from the desktop.
  expect(pending).toContain('fixed inset-x-0 bottom-0');
  expect(pending).not.toMatch(/data-status-bar-mode="with-toolbar"[^>]*style=/);
  // Hidden until the pointer reaches the top edge and the toolbar shows.
  expect(pending).toContain('translate-y-full');
  expect(pending).not.toContain('aria-keyshortcuts');
  expect(pending).toContain('Keep shown');
  expect(pending).toContain('data-local-input="true"');
  useSessionChrome.getInitialState().visible = true;
  expect(render('/session/opaque-id')).toContain('translate-y-0');
  useSessionStore.getInitialState().sessions = new Map([['session-id', { id: 'session-id', connectionId: 'opaque-id',
    connectionName: '', host: '', status: 'connected', fps: 0, latency: 0, bandwidth: 0, width: 940, height: 736,
    reconnectAttempts: 0, maxReconnectAttempts: 5 }]]);
  useSessionStore.getInitialState().activeSessionId = 'session-id';
  expect(render('/session/session-id')).toContain('Connected to Build host - 940x736');
  for (const path of ['/', '/settings', '/session/stale']) {
    expect(render(path)).toContain('data-status-bar-mode="visible"');
    expect(render(path)).not.toContain('fixed inset-x-0');
  }
  useConnectionStore.getInitialState().connections = [{ ...build, hostProfile: { ...DEFAULT_HOST_PROFILE, bottomBar: 'always' } }];
  expect(render('/session/session-id')).toContain('data-status-bar-mode="visible"');
  expect(render('/session/session-id')).toContain('style="height:28px"');
  expect(render('/session/session-id')).toContain('>Hide</span>');
  expect(render('/')).not.toContain('>Hide</span>');
});
