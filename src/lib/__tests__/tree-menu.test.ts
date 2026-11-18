import { describe, expect, it } from 'vitest';

import type { NodeId } from '../../types';
import { TREE_KEYBOARD_SCENARIOS } from '../ux-tree-states';
import {
  closeContextMenu,
  closedContextMenu,
  contextMenuItems,
  menuActionFor,
  openContextMenu,
} from '../tree-menu';

const con = (id: string): NodeId => ({ kind: 'connection', id });
const fol = (id: string): NodeId => ({ kind: 'folder', id });
const lib = (id: string): NodeId => ({ kind: 'library', id });

// ── Req 10.7 item sets (+ the 10.14 keyboard Move entry) ─────────────

describe('contextMenuItems (Req 10.7, pinned sets)', () => {
  it('offers server identity review on a saved connection', () => {
    expect(contextMenuItems('connection')).toEqual([
      'Connect',
      'Test credentials',
      'Review server identity',
      'Edit',
      'Duplicate',
      'Move…',
      'Delete',
    ]);
  });

  it('offers New Connection, New Folder, Rename, Move…, Delete for folders', () => {
    expect(contextMenuItems('folder')).toEqual([
      'New Connection',
      'New Folder',
      'Rename',
      'Move…',
      'Delete',
    ]);
  });

  it('offers New Connection, New Folder, Rename, Delete for libraries (not movable)', () => {
    expect(contextMenuItems('library')).toEqual([
      'New Connection',
      'New Folder',
      'Rename',
      'Delete',
    ]);
  });

  it('matches the Shift+F10 menu expectations pinned in TREE_KEYBOARD_SCENARIOS', () => {
    for (const kind of ['connection', 'folder', 'library'] as const) {
      const scenario = TREE_KEYBOARD_SCENARIOS.find(
        (s) => s.from.kind === kind && s.expectContextMenuItems !== undefined
      );
      expect(scenario, `pinned Shift+F10 scenario for ${kind}`).toBeDefined();
      expect(contextMenuItems(kind)).toEqual(scenario!.expectContextMenuItems);
    }
  });

  it('returns a fresh array per call (render layers may not mutate the pins)', () => {
    const a = contextMenuItems('folder');
    const b = contextMenuItems('folder');
    expect(a).not.toBe(b);
    expect(a).toEqual(b);
  });
});

describe('menuActionFor', () => {
  it('maps every pinned item label to its action', () => {
    expect(menuActionFor('Connect')).toBe('connect');
    expect(menuActionFor('Test credentials')).toBe('test-credentials');
    expect(menuActionFor('Review server identity')).toBe('review-server-identity');
    expect(menuActionFor('Edit')).toBe('edit');
    expect(menuActionFor('Duplicate')).toBe('duplicate');
    expect(menuActionFor('Move…')).toBe('move');
    expect(menuActionFor('Delete')).toBe('delete');
    expect(menuActionFor('New Connection')).toBe('new-connection');
    expect(menuActionFor('New Folder')).toBe('new-folder');
    expect(menuActionFor('Rename')).toBe('rename');
  });

  it('rejects unknown labels', () => {
    expect(() => menuActionFor('Not an item')).toThrow(/unknown context menu item/i);
  });
});

// ── Shift+F10 focus contract (ux-acceptance.md Row 1, Req 10.13/10.14) ──

describe('context menu focus state machine', () => {
  it('starts closed with no focus target', () => {
    expect(closedContextMenu()).toEqual({
      open: false,
      invokedFrom: null,
      focus: { target: 'tree-node', node: null },
    });
  });

  it('opening via Shift+F10 moves focus into the menu and remembers the invoking node', () => {
    const node = con('conn-web-prod');
    expect(openContextMenu(node)).toEqual({
      open: true,
      invokedFrom: node,
      focus: { target: 'context-menu', node },
    });
  });

  it('dismissing the menu returns focus to the invoking tree node', () => {
    const node = fol('folder-prod');
    const open = openContextMenu(node);
    const closed = closeContextMenu(open, 'dismiss');
    expect(closed.open).toBe(false);
    expect(closed.focus).toEqual({ target: 'tree-node', node });
    expect(closed.invokedFrom).toBe(node);
  });

  it('selecting a menu item also returns focus to the invoking tree node', () => {
    const node = lib('lib-team');
    const closed = closeContextMenu(openContextMenu(node), 'select');
    expect(closed.open).toBe(false);
    expect(closed.focus).toEqual({ target: 'tree-node', node });
  });

  it('closing an already-closed menu keeps a null focus node', () => {
    const closed = closeContextMenu(closedContextMenu(), 'dismiss');
    expect(closed).toEqual({
      open: false,
      invokedFrom: null,
      focus: { target: 'tree-node', node: null },
    });
  });
});
