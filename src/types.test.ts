import { describe, it, expect } from 'vitest';
import type { ConnectionConfig, SessionInfo, AppSettings } from './types';

describe('Type definitions', () => {
  it('ConnectionConfig has required fields', () => {
    const config: ConnectionConfig = {
      id: '1',
      name: 'Test',
      host: '192.168.1.1',
      port: 3389,
      username: 'admin',
      domain: '',
      colorAccent: null,
      tags: [],
      displayWidth: null,
      displayHeight: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    };
    expect(config.id).toBe('1');
    expect(config.colorAccent).toBeNull();
    expect(config.displayWidth).toBeNull();
  });

  it('ConnectionConfig carries v2 ownership and the inert mock flag (task 6.6)', () => {
    const config: ConnectionConfig = {
      id: '2',
      name: 'Filed',
      host: 'filed.invalid',
      port: 3389,
      username: 'admin',
      domain: '',
      colorAccent: null,
      tags: [],
      displayWidth: null,
      displayHeight: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      libraryId: 'lib-1',
      folderId: 'folder-1',
      mock: false,
    };
    expect(config.libraryId).toBe('lib-1');
    expect(config.folderId).toBe('folder-1');
    expect(config.mock).toBe(false);
  });

  it('SessionInfo has required fields', () => {
    const session: SessionInfo = {
      id: 'session-1',
      connectionId: 'conn-1',
      connectionName: 'Test Server',
      host: '192.168.1.1',
      status: 'connected',
      fps: 30,
      latency: 15,
      bandwidth: 1024,
      width: 1920,
      height: 1080,
      reconnectAttempts: 0,
      maxReconnectAttempts: 5,
    };
    expect(session.status).toBe('connected');
    expect(session.fps).toBe(30);
  });

  it('AppSettings has required fields', () => {
    const settings: AppSettings = {
      theme: 'dark',
      defaultPort: 3389,
      autoReconnect: true,
      showPerformanceHud: false,
      sidebarCollapsed: false,
      defaultResolution: '1920x1080',
      colorDepth: 32,
      quality: 80,
      reconnectTimeout: 5000,
      connectionTimeout: 10000,
    };
    expect(settings.theme).toBe('dark');
    expect(settings.defaultPort).toBe(3389);
  });
});
