import { describe, expect, it } from 'vitest';

import type { Folder, NodeId } from '../../types';
import {
  FILTER_STATE_PINS,
  MOVE_STATE_PINS,
  RAIL_STATE_PINS,
  SESSION_STATUS_TEXT,
  TREE_AGGREGATE_STATUS_TEXT,
  TREE_A11Y_STATE,
  TREE_KEYBOARD_RULES,
  TREE_KEYBOARD_SCENARIOS,
  TREE_SAVED_EXPANSION_DEFAULT,
  UX_ACCEPTANCE_TREE,
  UX_CONNECTION_TAGS,
  UX_SESSION_STATUSES,
} from '../ux-tree-states';
import type { TreeKey, TreeKeyRule } from '../ux-tree-states';

// ── Test-local derivations from the fixture tree (independent checks) ──

const lib = (id: string): NodeId => ({ kind: 'library', id });
const fol = (id: string): NodeId => ({ kind: 'folder', id });
const con = (id: string): NodeId => ({ kind: 'connection', id });

function must<T>(value: T | undefined, what: string): T {
  if (value === undefined) throw new Error(`missing ${what}`);
  return value;
}

/** Depth-first render order: folders before connections per container, honoring expansion. */
function flattenVisible(expanded: Record<string, boolean>): NodeId[] {
  const out: NodeId[] = [];
  const walkFolder = (folderId: string): void => {
    out.push(fol(folderId));
    if (!expanded[folderId]) return;
    for (const f of UX_ACCEPTANCE_TREE.folders.filter((x) => x.parentId === folderId)) walkFolder(f.id);
    for (const c of UX_ACCEPTANCE_TREE.connections.filter((x) => x.folderId === folderId)) out.push(con(c.id));
  };
  for (const l of UX_ACCEPTANCE_TREE.libraries) {
    out.push(lib(l.id));
    if (!expanded[l.id]) continue;
    for (const f of UX_ACCEPTANCE_TREE.folders.filter((x) => x.libraryId === l.id && x.parentId === null)) walkFolder(f.id);
    for (const c of UX_ACCEPTANCE_TREE.connections.filter((x) => x.libraryId === l.id && x.folderId === null)) out.push(con(c.id));
  }
  return out;
}

/** Folder ancestor chain (root-first), including the folder itself. */
function folderAncestorChain(folderId: string): Folder[] {
  const chain: Folder[] = [];
  let currentId: string | null = folderId;
  while (currentId !== null) {
    const id: string = currentId;
    const folder = must(UX_ACCEPTANCE_TREE.folders.find((x) => x.id === id), `folder ${id}`);
    chain.unshift(folder);
    currentId = folder.parentId;
  }
  return chain;
}

/** All ancestor nodes of a node, nearest parent last (Req 10.9 ancestor paths). */
function ancestorsOf(node: NodeId): NodeId[] {
  if (node.kind === 'library') return [];
  if (node.kind === 'connection') {
    const c = must(UX_ACCEPTANCE_TREE.connections.find((x) => x.id === node.id), `connection ${node.id}`);
    const chain: NodeId[] = [lib(c.libraryId)];
    if (c.folderId !== null) for (const f of folderAncestorChain(c.folderId)) chain.push(fol(f.id));
    return chain;
  }
  const ownChain = folderAncestorChain(node.id);
  const chain: NodeId[] = [lib(ownChain[0].libraryId)];
  for (const f of ownChain.slice(0, -1)) chain.push(fol(f.id));
  return chain;
}

/** Every descendant connection of a container (folder or library). */
function descendantConnectionIds(containerId: string): string[] {
  if (UX_ACCEPTANCE_TREE.libraries.some((l) => l.id === containerId)) {
    return UX_ACCEPTANCE_TREE.connections.filter((c) => c.libraryId === containerId).map((c) => c.id);
  }
  const folderIds = new Set<string>([containerId]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const f of UX_ACCEPTANCE_TREE.folders) {
      if (f.parentId !== null && folderIds.has(f.parentId) && !folderIds.has(f.id)) {
        folderIds.add(f.id);
        grew = true;
      }
    }
  }
  return UX_ACCEPTANCE_TREE.connections.filter((c) => c.folderId !== null && folderIds.has(c.folderId)).map((c) => c.id);
}

const statusOf = (connectionId: string) => UX_SESSION_STATUSES[connectionId] ?? 'disconnected';

function expectedAggregate(containerId: string): string {
  const descendants = descendantConnectionIds(containerId);
  const connected = descendants.filter((id) => statusOf(id) === 'connected').length;
  return `${connected} of ${descendants.length} connected`;
}

/** Nodes a query hits: name for all nodes; host/tags for connections (Req 10.9). */
function computeMatches(query: string): Set<string> {
  const q = query.toLowerCase();
  const ids = new Set<string>();
  for (const l of UX_ACCEPTANCE_TREE.libraries) if (l.name.toLowerCase().includes(q)) ids.add(l.id);
  for (const f of UX_ACCEPTANCE_TREE.folders) if (f.name.toLowerCase().includes(q)) ids.add(f.id);
  for (const c of UX_ACCEPTANCE_TREE.connections) {
    const byName = c.name.toLowerCase().includes(q);
    const byHost = c.host.toLowerCase().includes(q);
    const byTags = (UX_CONNECTION_TAGS[c.id] ?? []).some((t) => t.toLowerCase().includes(q));
    if (byName || byHost || byTags) ids.add(c.id);
  }
  return ids;
}

/** Matches restricted to a single source, for pinning matchedOn semantics. */
function computeMatchesBy(query: string, source: 'name' | 'host'): Set<string> {
  const q = query.toLowerCase();
  const ids = new Set<string>();
  if (source === 'name') {
    for (const l of UX_ACCEPTANCE_TREE.libraries) if (l.name.toLowerCase().includes(q)) ids.add(l.id);
    for (const f of UX_ACCEPTANCE_TREE.folders) if (f.name.toLowerCase().includes(q)) ids.add(f.id);
    for (const c of UX_ACCEPTANCE_TREE.connections) if (c.name.toLowerCase().includes(q)) ids.add(c.id);
  } else {
    for (const c of UX_ACCEPTANCE_TREE.connections) if (c.host.toLowerCase().includes(q)) ids.add(c.id);
  }
  return ids;
}

const ALL_KEYS: TreeKey[] = ['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'Enter', 'Shift+F10'];

const ruleFor = (key: TreeKey, kind: NodeId['kind'], expanded: boolean): TreeKeyRule =>
  must(
    TREE_KEYBOARD_RULES.find(
      (r) => r.key === key && r.kind === kind && (r.when === undefined || r.when === (expanded ? 'expanded' : 'collapsed'))
    ),
    `rule for ${key} on ${kind} (${expanded ? 'expanded' : 'collapsed'})`
  );

describe('ux-tree-states: fixture tree integrity', () => {
  it('every folder references an existing library and parent', () => {
    const libraryIds = new Set(UX_ACCEPTANCE_TREE.libraries.map((l) => l.id));
    const folderIds = new Set(UX_ACCEPTANCE_TREE.folders.map((f) => f.id));
    for (const f of UX_ACCEPTANCE_TREE.folders) {
      expect(libraryIds.has(f.libraryId)).toBe(true);
      expect(f.parentId === null || folderIds.has(f.parentId)).toBe(true);
    }
  });

  it('every connection references an existing library and folder (or library root)', () => {
    const libraryIds = new Set(UX_ACCEPTANCE_TREE.libraries.map((l) => l.id));
    const folderIds = new Set(UX_ACCEPTANCE_TREE.folders.map((f) => f.id));
    for (const c of UX_ACCEPTANCE_TREE.connections) {
      expect(libraryIds.has(c.libraryId)).toBe(true);
      expect(c.folderId === null || folderIds.has(c.folderId)).toBe(true);
    }
  });

  it('covers every live status plus the no-session case (Req 10.4)', () => {
    const used = new Set(Object.values(UX_SESSION_STATUSES));
    for (const status of ['disconnected', 'connecting', 'connected', 'reconnecting', 'error'] as const) {
      expect(used.has(status)).toBe(true);
    }
    // …and at least one connection without any session.
    const withSession = new Set(Object.keys(UX_SESSION_STATUSES));
    expect(UX_ACCEPTANCE_TREE.connections.some((c) => !withSession.has(c.id))).toBe(true);
  });

  it('includes a library-root connection (the 10.10 migration shape)', () => {
    expect(UX_ACCEPTANCE_TREE.connections.some((c) => c.folderId === null)).toBe(true);
  });
});

describe('SESSION_STATUS_TEXT (Req 10.4/10.14)', () => {
  it('maps every SessionStatus to a distinct non-empty text', () => {
    const texts = Object.values(SESSION_STATUS_TEXT);
    expect(texts).toHaveLength(5);
    for (const text of texts) expect(text.length).toBeGreaterThan(0);
    expect(new Set(texts).size).toBe(5);
  });
});

describe('TREE_A11Y_STATE: annotated visible tree (Req 10.3/10.14)', () => {
  it('is exactly the render order derived from the saved expansion', () => {
    const expected = flattenVisible(TREE_SAVED_EXPANSION_DEFAULT);
    expect(TREE_A11Y_STATE.map((n) => n.node.id)).toEqual(expected.map((n) => n.id));
  });

  it('exposes name, level, kind, expansion and selection on every visible node (10.14)', () => {
    for (const entry of TREE_A11Y_STATE) {
      expect(entry.name.length).toBeGreaterThan(0);
      expect(entry.kind).toBe(entry.node.kind);
      expect(entry.level).toBeGreaterThan(0);
      if (entry.kind === 'connection') expect(entry.expanded).toBeNull();
      else expect(typeof entry.expanded).toBe('boolean');
    }
  });

  it('levels are structural: libraries at 1, children one deeper', () => {
    const byId = new Map(TREE_A11Y_STATE.map((n) => [n.node.id, n]));
    for (const entry of TREE_A11Y_STATE) {
      if (entry.kind === 'library') {
        expect(entry.level).toBe(1);
      } else {
        const parents = ancestorsOf(entry.node);
        const parent = parents[parents.length - 1];
        expect(entry.level).toBe(must(byId.get(parent.id), `visible parent ${parent.id}`).level + 1);
      }
    }
  });

  it('exactly one node is selected and it exists in the tree', () => {
    const selected = TREE_A11Y_STATE.filter((n) => n.selected);
    expect(selected).toHaveLength(1);
    expect(flattenVisible(TREE_SAVED_EXPANSION_DEFAULT).some((n) => n.id === selected[0].node.id)).toBe(true);
  });

  it('expansion flags mirror the saved expansion', () => {
    for (const entry of TREE_A11Y_STATE) {
      if (entry.kind !== 'connection') {
        expect(entry.expanded).toBe(TREE_SAVED_EXPANSION_DEFAULT[entry.node.id]);
      }
    }
  });

  it('every node carries a non-color textual status (10.4/10.14)', () => {
    for (const entry of TREE_A11Y_STATE) expect(entry.statusText.length).toBeGreaterThan(0);
  });

  it('connection status text is the session-status mapping; no session reads Disconnected', () => {
    for (const entry of TREE_A11Y_STATE) {
      if (entry.kind !== 'connection') continue;
      expect(entry.statusText).toBe(SESSION_STATUS_TEXT[statusOf(entry.node.id)]);
    }
  });

  it('container status text is the descendant aggregate `<connected> of <total> connected`', () => {
    for (const [containerId, text] of Object.entries(TREE_AGGREGATE_STATUS_TEXT)) {
      expect(text).toBe(expectedAggregate(containerId));
    }
    for (const entry of TREE_A11Y_STATE) {
      if (entry.kind !== 'connection') expect(entry.statusText).toBe(TREE_AGGREGATE_STATUS_TEXT[entry.node.id]);
    }
  });

  it('hasChildren matches the tree structure', () => {
    for (const entry of TREE_A11Y_STATE) {
      if (entry.kind === 'library') {
        const has =
          UX_ACCEPTANCE_TREE.folders.some((f) => f.libraryId === entry.node.id) ||
          UX_ACCEPTANCE_TREE.connections.some((c) => c.libraryId === entry.node.id);
        expect(entry.hasChildren).toBe(has);
      } else if (entry.kind === 'folder') {
        const has =
          UX_ACCEPTANCE_TREE.folders.some((f) => f.parentId === entry.node.id) ||
          UX_ACCEPTANCE_TREE.connections.some((c) => c.folderId === entry.node.id);
        expect(entry.hasChildren).toBe(has);
      } else {
        expect(entry.hasChildren).toBe(false);
      }
    }
  });
});

describe('TREE_KEYBOARD_RULES: roving-focus decision table (Req 10.13)', () => {
  it('covers every key for every node kind — containers split by expansion', () => {
    for (const key of ALL_KEYS) {
      for (const kind of ['library', 'folder', 'connection'] as const) {
        const rules = TREE_KEYBOARD_RULES.filter((r) => r.key === key && r.kind === kind);
        if (kind === 'connection') {
          expect(rules, `${key} on connection`).toHaveLength(1);
          expect(rules[0].when).toBeUndefined();
        } else if (key === 'ArrowLeft' || key === 'ArrowRight') {
          expect(rules, `${key} on ${kind}`).toHaveLength(2);
          const whens = rules.map((r) => r.when).sort();
          expect(whens).toEqual(['collapsed', 'expanded']);
        } else {
          expect(rules, `${key} on ${kind}`).toHaveLength(1);
          expect(rules[0].when).toBeUndefined();
        }
      }
    }
  });

  it('every outcome keeps focus visible: tree, context menu, or opened tab (10.13)', () => {
    for (const rule of TREE_KEYBOARD_RULES) {
      expect(['tree-node', 'context-menu', 'opened-session-tab']).toContain(rule.focusAfter);
    }
    for (const rule of TREE_KEYBOARD_RULES) {
      if (rule.key === 'Shift+F10') expect(rule.focusAfter).toBe('context-menu');
      else if (rule.key === 'Enter' && rule.kind === 'connection') expect(rule.focusAfter).toBe('opened-session-tab');
      else expect(rule.focusAfter).toBe('tree-node');
    }
  });

  it('Enter selects a dashboard for containers and connects for connections (10.13)', () => {
    expect(ruleFor('Enter', 'library', true).outcome).toBe('select-dashboard');
    expect(ruleFor('Enter', 'folder', false).outcome).toBe('select-dashboard');
    expect(ruleFor('Enter', 'connection', false).outcome).toBe('connect');
  });
});

describe('TREE_KEYBOARD_SCENARIOS: concrete transitions on the fixture tree', () => {
  const visibleIds = (expanded: Record<string, boolean>) => flattenVisible(expanded).map((n) => n.id);

  it('every scenario starts from a node visible under its precondition', () => {
    for (const s of TREE_KEYBOARD_SCENARIOS) {
      const base = { ...TREE_SAVED_EXPANSION_DEFAULT, ...s.preconditionExpansion };
      expect(visibleIds(base)).toContain(s.from.id);
    }
  });

  it('each scenario satisfies its keyboard rule outcome, derived from the tree', () => {
    for (const s of TREE_KEYBOARD_SCENARIOS) {
      const base = { ...TREE_SAVED_EXPANSION_DEFAULT, ...s.preconditionExpansion };
      const rule = ruleFor(s.key, s.from.kind, base[s.from.id] ?? false);

      const order = visibleIds(base);
      const idx = order.indexOf(s.from.id);
      let expectedFocusId: string;
      switch (rule.outcome) {
        case 'focus-previous-visible-node':
          expectedFocusId = order[Math.max(idx - 1, 0)];
          break;
        case 'focus-next-visible-node':
          expectedFocusId = order[Math.min(idx + 1, order.length - 1)];
          break;
        case 'focus-first-visible-node':
          expectedFocusId = order[0];
          break;
        case 'focus-last-visible-node':
          expectedFocusId = order[order.length - 1];
          break;
        case 'focus-first-child': {
          const afterExpand = visibleIds({ ...base, [s.from.id]: true });
          expectedFocusId = afterExpand[afterExpand.indexOf(s.from.id) + 1];
          break;
        }
        case 'focus-parent': {
          const parents = ancestorsOf(s.from);
          expectedFocusId = parents[parents.length - 1].id;
          break;
        }
        default:
          expectedFocusId = s.from.id;
      }
      expect(s.expectFocus.id, s.scenario).toBe(expectedFocusId);

      // Expansion changes are exactly what the rule prescribes.
      if (rule.outcome === 'expand-focused-node' || rule.outcome === 'collapse-focused-node') {
        expect(s.expectExpansionChange, s.scenario).toEqual({
          nodeId: s.from.id,
          expanded: rule.outcome === 'expand-focused-node',
        });
      } else {
        expect(s.expectExpansionChange, s.scenario).toBeUndefined();
      }

      // Enter semantics (10.13).
      if (rule.outcome === 'select-dashboard') {
        expect(s.expectSelectDashboard, s.scenario).toEqual(s.from);
        expect(s.expectConnect, s.scenario).toBeUndefined();
      }
      if (rule.outcome === 'connect') {
        expect(s.expectConnect, s.scenario).toBe(s.from.id);
        expect(s.expectSelectDashboard, s.scenario).toBeUndefined();
      }
    }
  });

  it('ArrowDown skips collapsed subtrees and clamps at the edges (10.13)', () => {
    const byScenario = (needle: string) =>
      must(TREE_KEYBOARD_SCENARIOS.find((s) => s.scenario.includes(needle)), `scenario "${needle}"`);
    expect(byScenario('skips the children').expectFocus).toEqual(fol('folder-lab'));
    expect(byScenario('last visible node clamps').expectFocus).toEqual(con('conn-quick'));
    expect(byScenario('first visible node clamps').expectFocus).toEqual(lib('lib-personal'));
  });

  it('Shift+F10 menus are the 10.7 sets plus Move for movable kinds (10.14)', () => {
    for (const s of TREE_KEYBOARD_SCENARIOS) {
      if (s.key !== 'Shift+F10') continue;
      const expected =
        s.from.kind === 'connection'
          ? ['Connect', 'Review server identity', 'Edit', 'Duplicate', 'Move…', 'Delete']
          : s.from.kind === 'folder'
            ? ['New Connection', 'New Folder', 'Rename', 'Move…', 'Delete']
            : ['New Connection', 'New Folder', 'Rename', 'Delete'];
      expect(s.expectContextMenuItems, s.scenario).toEqual(expected);
    }
  });

  it('pins every key at least once (10.13 completeness)', () => {
    const used = new Set(TREE_KEYBOARD_SCENARIOS.map((s) => s.key));
    for (const key of ALL_KEYS) expect(used.has(key), key).toBe(true);
  });
});

describe('MOVE_STATE_PINS (Req 10.8/10.14/10.15)', () => {
  it('pins both triggers — keyboard dialog and drag-and-drop — for success and failure', () => {
    const triggers = new Set(MOVE_STATE_PINS.map((p) => p.trigger));
    expect(triggers).toEqual(new Set(['keyboard-move-dialog', 'drag-drop']));
    for (const trigger of triggers) {
      expect(MOVE_STATE_PINS.some((p) => p.trigger === trigger && p.result === 'succeeds')).toBe(true);
      expect(MOVE_STATE_PINS.some((p) => p.trigger === trigger && p.result === 'fails')).toBe(true);
    }
  });

  it('success: the node lands under the destination and focus follows it', () => {
    for (const pin of MOVE_STATE_PINS.filter((p) => p.result === 'succeeds')) {
      expect(pin.locationAfter, pin.scenario).toBe('under-destination');
      expect(pin.focusAfter).toEqual(pin.actor);
      expect(pin.persistedTreeUnchanged).toBe(false);
      expect(pin.message).toBeUndefined();
    }
  });

  it('failure: previous location, persisted tree and focus restored, message shown (10.15)', () => {
    for (const pin of MOVE_STATE_PINS.filter((p) => p.result === 'fails')) {
      expect(pin.locationAfter).toBe('previous-location');
      expect(pin.persistedTreeUnchanged).toBe(true);
      expect(pin.focusAfter).toEqual(pin.actor);
      expect(pin.failureReason).toBeDefined();
      expect(must(pin.message, `message for "${pin.scenario}"`).length).toBeGreaterThan(0);
    }
  });

  it('actors and destinations exist in the fixture tree', () => {
    const ids = new Set([
      ...UX_ACCEPTANCE_TREE.libraries.map((l) => l.id),
      ...UX_ACCEPTANCE_TREE.folders.map((f) => f.id),
      ...UX_ACCEPTANCE_TREE.connections.map((c) => c.id),
    ]);
    for (const pin of MOVE_STATE_PINS) {
      expect(ids.has(pin.actor.id)).toBe(true);
      expect(ids.has(pin.destination.id)).toBe(true);
    }
  });

  it('the subtree-validation failure really moves a folder into its own subtree', () => {
    const pin = must(
      MOVE_STATE_PINS.find((p) => p.failureReason === 'destination-inside-actor-subtree'),
      'subtree failure pin'
    );
    // folder-vms sits inside folder-work, so its subtree connections prove the nesting.
    expect(descendantConnectionIds(pin.actor.id)).toContain('conn-web-dev');
    expect(folderAncestorChain(pin.destination.id).some((f) => f.id === pin.actor.id)).toBe(true);
  });
});

describe('FILTER_STATE_PINS (Req 10.9)', () => {
  it('matched sets are exactly the name/host/tag matches computed from the tree', () => {
    for (const pin of FILTER_STATE_PINS) {
      const expected = computeMatches(pin.query);
      expect(new Set(pin.matched.map((n) => n.id)), pin.query).toEqual(expected);
    }
  });

  it('pins all three match sources: name, host and tags', () => {
    const sources = new Set(FILTER_STATE_PINS.map((p) => p.matchedOn));
    expect(sources).toEqual(new Set(['name', 'host', 'tags']));
    // The tag pin must match through tags only.
    const tagPin = must(FILTER_STATE_PINS.find((p) => p.matchedOn === 'tags'), 'tag pin');
    expect(computeMatchesBy(tagPin.query, 'name').size).toBe(0);
    expect(computeMatchesBy(tagPin.query, 'host').size).toBe(0);
  });

  it('required ancestors are exactly the matched nodes’ ancestor paths', () => {
    for (const pin of FILTER_STATE_PINS) {
      const expected = new Set<string>();
      for (const m of pin.matched) for (const a of ancestorsOf(m)) expected.add(a.id);
      expect(new Set(pin.requiredAncestors.map((n) => n.id)), pin.query).toEqual(expected);
    }
  });

  it('visible-during-filter is matches plus ancestors in tree order', () => {
    const allExpanded = Object.fromEntries(
      Object.keys(TREE_SAVED_EXPANSION_DEFAULT).map((id) => [id, true])
    );
    for (const pin of FILTER_STATE_PINS) {
      const keep = new Set([...pin.matched.map((n) => n.id), ...pin.requiredAncestors.map((n) => n.id)]);
      const fullOrder = flattenVisible(allExpanded);
      expect(pin.visibleDuringFilter.map((n) => n.id), pin.query).toEqual(
        fullOrder.filter((n) => keep.has(n.id)).map((n) => n.id)
      );
    }
  });

  it('temporary expansion never overwrites the saved state, and clearing restores it exactly', () => {
    for (const pin of FILTER_STATE_PINS) {
      expect(pin.savedExpansionDuringFilter, pin.scenario).toEqual(pin.savedExpansionBefore);
      expect(pin.restoredExpansionAfterClear, pin.scenario).toEqual(pin.savedExpansionBefore);
    }
  });

  it('demonstrates divergence: a saved-collapsed ancestor expands only while filtering', () => {
    const nested = must(FILTER_STATE_PINS.find((p) => p.query === 'web'), 'nested filter pin');
    expect(nested.temporaryExpansion['folder-vms']).toBe(true);
    expect(TREE_SAVED_EXPANSION_DEFAULT['folder-vms']).toBe(false);
    expect(nested.savedExpansionDuringFilter['folder-vms']).toBe(false);
  });

  it('no-match clears the tree and pins an explicit empty-state text', () => {
    const none = must(FILTER_STATE_PINS.find((p) => p.matched.length === 0), 'no-match pin');
    expect(none.visibleDuringFilter).toEqual([]);
    expect(none.requiredAncestors).toEqual([]);
    expect(must(none.emptyStateText, 'empty-state text').length).toBeGreaterThan(0);
  });
});

describe('RAIL_STATE_PINS (Req 10.11/10.4/10.14)', () => {
  it('pins both the expanded sidebar and the collapsed rail', () => {
    expect(RAIL_STATE_PINS.map((p) => p.scenario)).toContain('expanded-sidebar');
    expect(RAIL_STATE_PINS.map((p) => p.scenario)).toContain('collapsed-rail');
    expect(new Set(RAIL_STATE_PINS.map((p) => p.sidebarCollapsed))).toEqual(new Set([false, true]));
  });

  it('the collapse control is named and flips its label (explicit collapse, 10.11)', () => {
    const names = RAIL_STATE_PINS.map((p) => p.toggleControlName);
    for (const name of names) expect(name.length).toBeGreaterThan(0);
    expect(new Set(names).size).toBe(2);
  });

  it('every library appears exactly once with a textual status (not color-only)', () => {
    for (const pin of RAIL_STATE_PINS) {
      expect(pin.entries.map((e) => e.library.id)).toEqual(UX_ACCEPTANCE_TREE.libraries.map((l) => l.id));
      for (const entry of pin.entries) {
        expect(entry.statusText, entry.library.id).toBe(expectedAggregate(entry.library.id));
        expect(entry.accessibleName).toBe(`${entry.name}, ${entry.statusText}`);
      }
    }
  });

  it('collapse preserves selection and expansion exactly (10.11)', () => {
    const expanded = must(RAIL_STATE_PINS.find((p) => p.scenario === 'expanded-sidebar'), 'expanded rail pin');
    const collapsed = must(RAIL_STATE_PINS.find((p) => p.scenario === 'collapsed-rail'), 'collapsed rail pin');
    expect(expanded.preserved).toEqual(collapsed.preserved);
    expect(expanded.preserved.expanded).toEqual(TREE_SAVED_EXPANSION_DEFAULT);
    expect(expanded.preserved.selectedNodeId).toEqual(con('conn-bastion'));
  });
});
