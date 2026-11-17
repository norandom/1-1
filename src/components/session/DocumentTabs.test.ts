import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, expect, it } from 'vitest';
import { DocumentTabs } from './DocumentTabs';
import { useConnectionStore } from '../../stores/connectionStore';
import { useSessionStore } from '../../stores/sessionStore';
import type { ConnectionConfig, SessionInfo } from '../../types';

afterEach(() => {
  useConnectionStore.getInitialState().connections = [];
  useSessionStore.getInitialState().sessions = new Map();
  useSessionStore.setState({ sessions: new Map() });
});

it('offers Resume on the paused session tab only', () => {
  const active = { id: 'a', connectionId: 'host-a', status: 'connected', idlePreviewState: 'live' } as SessionInfo;
  const idle = { ...active, id: 'b', connectionId: 'host-b', idlePreviewState: 'suppressed' } as SessionInfo;
  useSessionStore.getInitialState().sessions = new Map([[active.id, active], [idle.id, idle]]);
  useConnectionStore.getInitialState().connections = [active, idle].map((s) => ({ id: s.connectionId, name: s.connectionId }) as ConnectionConfig);
  const render = (path: string) => renderToStaticMarkup(createElement(MemoryRouter, { initialEntries: [path] }, createElement(DocumentTabs)));
  expect(render('/session/b')).toContain('Resume live updates');
  expect(render('/session/a')).not.toContain('Resume live updates');
  expect(render('/')).not.toContain('Resume live updates');
});

it('reserves the real document strip before first connect, but not on empty dashboards or stale routes', () => {
  // SSR reads Zustand's initial snapshot; seed that snapshot explicitly.
  useConnectionStore.getInitialState().connections = [{ id: 'first', name: 'First host', host: 'test.invalid',
    port: 3389, username: '', domain: '', colorAccent: null, tags: [],
    displayWidth: null, displayHeight: null, fontSmoothing: true, createdAt: '', updatedAt: '' }];
  const render = (path: string) => renderToStaticMarkup(createElement(MemoryRouter, { initialEntries: [path] }, createElement(DocumentTabs)));
  expect(render('/session/first')).toContain('role="tablist"');
  expect(render('/session/first')).toContain('Connecting to First host');
  expect(render('/')).toBe('');
  expect(render('/session/missing')).toBe('');
});
