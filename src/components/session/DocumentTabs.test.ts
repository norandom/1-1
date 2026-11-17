import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, expect, it } from 'vitest';
import { DocumentTabs } from './DocumentTabs';
import { useConnectionStore } from '../../stores/connectionStore';
import { useSessionStore } from '../../stores/sessionStore';

afterEach(() => {
  useConnectionStore.getInitialState().connections = [];
  useSessionStore.setState({ sessions: new Map() });
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
