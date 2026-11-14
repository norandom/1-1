import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import type { ConnectionSummary, Folder, Library, LibraryTree } from '../../types';

import {
  createFolder,
  createLibrary,
  getLibraryStatus,
  listDescendants,
  listTree,
  moveNode,
  reloadLibraryStore,
  renameFolder,
  renameLibrary,
} from '../tauri';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

beforeEach(() => {
  invokeMock.mockReset();
});

describe('library wrappers (task 6.4, Req 10.1/10.2/10.3/10.5)', () => {
  it('createLibrary marshals the name and returns the camelCase library', async () => {
    const library: Library = {
      id: '6f1c2f76-79cf-4c85-9454-44b16a00cd64',
      name: 'Work',
      createdAt: '2026-11-14T10:00:00Z',
      updatedAt: '2026-11-14T10:00:00Z',
    };
    invokeMock.mockResolvedValueOnce(library);

    await expect(createLibrary('Work')).resolves.toEqual(library);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('create_library', { name: 'Work' });
  });

  it('renameLibrary marshals id and name', async () => {
    invokeMock.mockResolvedValueOnce(null);
    await renameLibrary('lib-1', 'Personal');
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('rename_library', {
      id: 'lib-1',
      name: 'Personal',
    });
  });

  it('createFolder marshals camelCase libraryId and parentId (null at library root)', async () => {
    const folder: Folder = {
      id: 'folder-1',
      libraryId: 'lib-1',
      parentId: null,
      name: 'Prod',
    };
    invokeMock.mockResolvedValueOnce(folder);

    await expect(createFolder('lib-1', null, 'Prod')).resolves.toEqual(folder);
    expect(invokeMock).toHaveBeenCalledWith('create_folder', {
      libraryId: 'lib-1',
      parentId: null,
      name: 'Prod',
    });
  });

  it('createFolder passes a nested parentId through', async () => {
    invokeMock.mockResolvedValueOnce(null);
    await createFolder('lib-1', 'folder-1', 'Europe');
    expect(invokeMock).toHaveBeenCalledWith('create_folder', {
      libraryId: 'lib-1',
      parentId: 'folder-1',
      name: 'Europe',
    });
  });

  it('renameFolder marshals id and name', async () => {
    invokeMock.mockResolvedValueOnce(null);
    await renameFolder('folder-1', 'EMEA');
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('rename_folder', {
      id: 'folder-1',
      name: 'EMEA',
    });
  });

  it('listTree invokes the read model without arguments', async () => {
    const tree: LibraryTree = {
      libraries: [],
      folders: [],
      connections: [],
    };
    invokeMock.mockResolvedValueOnce(tree);
    await expect(listTree()).resolves.toEqual(tree);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('list_tree');
  });

  it('listDescendants marshals the node kind and id', async () => {
    const summaries: ConnectionSummary[] = [
      {
        id: 'conn-1',
        name: 'web-1',
        host: 'web-1.invalid',
        libraryId: 'lib-1',
        folderId: 'folder-1',
      },
    ];
    invokeMock.mockResolvedValueOnce(summaries);
    await expect(listDescendants('folder', 'folder-1')).resolves.toEqual(summaries);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('list_descendants', {
      kind: 'folder',
      id: 'folder-1',
    });
  });

  it('getLibraryStatus and reloadLibraryStore hit their commands', async () => {
    invokeMock.mockResolvedValueOnce({ status: 'ready' });
    await expect(getLibraryStatus()).resolves.toEqual({ status: 'ready' });
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('get_library_status');

    invokeMock.mockReset();
    invokeMock.mockResolvedValueOnce({ status: 'writeBlocked', reason: 'r', instructions: 'i' });
    await expect(reloadLibraryStore()).resolves.toEqual({
      status: 'writeBlocked',
      reason: 'r',
      instructions: 'i',
    });
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('reload_library_store');
  });

  it('CRUD wrappers reject instead of swallowing command errors', async () => {
    invokeMock.mockRejectedValueOnce(new Error('store is write-blocked'));
    await expect(createLibrary('Work')).rejects.toThrow('store is write-blocked');
  });
});

describe('move wrapper (task 6.5, Req 10.8/10.15)', () => {
  it('moveNode marshals the node and target kind/id pairs', async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await expect(moveNode('folder', 'folder-1', 'library', 'lib-2')).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('move_node', {
      kind: 'folder',
      id: 'folder-1',
      targetKind: 'library',
      targetId: 'lib-2',
    });
  });

  it('moveNode marshals cross-library connection moves', async () => {
    invokeMock.mockResolvedValueOnce(null);
    await moveNode('connection', 'conn-1', 'folder', 'folder-9');
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('move_node', {
      kind: 'connection',
      id: 'conn-1',
      targetKind: 'folder',
      targetId: 'folder-9',
    });
  });

  it('moveNode rejects instead of swallowing command errors (Req 10.15)', async () => {
    invokeMock.mockRejectedValueOnce(
      new Error('library operation rejected: folder parent chain is cyclic at folder-1')
    );
    await expect(moveNode('folder', 'folder-1', 'folder', 'folder-3')).rejects.toThrow(
      'folder parent chain is cyclic at folder-1'
    );
  });
});
