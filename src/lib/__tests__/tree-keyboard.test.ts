import { describe, expect, it } from 'vitest';

import type { NodeId } from '../../types';
import {
  TREE_A11Y_STATE,
  TREE_KEYBOARD_RULES,
  TREE_KEYBOARD_SCENARIOS,
  TREE_SAVED_EXPANSION_DEFAULT,
  UX_ACCEPTANCE_TREE,
} from '../ux-tree-states';
import type { TreeKey, TreeKeyRule } from '../ux-tree-states';
import { applyTreeKey, resolveTreeKeyRule, visibleTreeOrder } from '../tree-keyboard';

const expansionFor = (overrides?: Record<string, boolean>): Record<string, boolean> => ({
  ...TREE_SAVED_EXPANSION_DEFAULT,
  ...(overrides ?? {}),
});

// ── Visible order (Req 10.3: children show/hide in place) ────────────

describe('visibleTreeOrder', () => {
  it('matches the pinned visible order (TREE_A11Y_STATE order)', () => {
    expect(visibleTreeOrder(UX_ACCEPTANCE_TREE, TREE_SAVED_EXPANSION_DEFAULT)).toEqual(
      TREE_A11Y_STATE.map((n) => n.node)
    );
  });

  it('skips the children of collapsed containers', () => {
    const order = visibleTreeOrder(UX_ACCEPTANCE_TREE, expansionFor({ 'folder-work': false }));
    const ids = order.map((n) => n.id);
    expect(ids).toContain('folder-work');
    expect(ids).not.toContain('folder-vms');
    expect(ids).not.toContain('conn-term');
  });
});

// ── All 22 pinned scenarios (Req 10.13) ──────────────────────────────

describe('TREE_KEYBOARD_SCENARIOS', () => {
  for (const scenario of TREE_KEYBOARD_SCENARIOS) {
    it(scenario.scenario, () => {
      const result = applyTreeKey({
        key: scenario.key,
        focus: scenario.from,
        tree: UX_ACCEPTANCE_TREE,
        expanded: expansionFor(scenario.preconditionExpansion),
      });

      expect(result.focus, 'focused node after the key').toEqual(scenario.expectFocus);

      if (scenario.expectExpansionChange) {
        expect(result.expansionChange).toEqual(scenario.expectExpansionChange);
      } else {
        expect(result.expansionChange, 'no unintended expansion change').toBeNull();
      }

      if (scenario.expectSelectDashboard) {
        expect(result.action).toEqual({
          type: 'select-dashboard',
          node: scenario.expectSelectDashboard,
        });
      } else if (scenario.expectConnect) {
        expect(result.action).toEqual({
          type: 'connect',
          node: { kind: 'connection', id: scenario.expectConnect },
        });
        // Rule pin: focus moves to the opened session tab (render layer's
        // concern; the roving tree focus itself stays on the connection).
        expect(result.focusAfter).toBe('opened-session-tab');
      } else if (scenario.expectContextMenuItems) {
        expect(result.action).toEqual({
          type: 'open-context-menu',
          node: scenario.from,
          items: scenario.expectContextMenuItems,
        });
        expect(result.focusAfter).toBe('context-menu');
      } else {
        expect(result.action, 'key moves focus only').toBeNull();
      }
    });
  }
});

// ── Rule resolution: all 28 pinned rules honored (Req 10.13) ─────────

describe('resolveTreeKeyRule / every pinned rule applies verbatim', () => {
  interface RuleCase {
    rule: TreeKeyRule;
    key: TreeKey;
    focus: NodeId;
    expanded: Record<string, boolean>;
  }

  const node = (kind: 'library' | 'folder' | 'connection', id: string): NodeId => ({ kind, id });
  const rules = (key: TreeKey): TreeKeyRule[] => TREE_KEYBOARD_RULES.filter((r) => r.key === key);

  const cases: RuleCase[] = [
    // ArrowUp/Down/Home/End: one per kind (expansion irrelevant).
    ...(['ArrowUp', 'ArrowDown', 'Home', 'End'] as const).flatMap((key) =>
      rules(key).map((rule) => ({
        rule,
        key,
        focus: node(rule.kind, rule.kind === 'library' ? 'lib-personal' : rule.kind === 'folder' ? 'folder-work' : 'conn-term'),
        expanded: expansionFor(),
      }))
    ),
    // ArrowRight: library collapsed → expand; library expanded → first child.
    { rule: rules('ArrowRight').find((r) => r.kind === 'library' && r.when === 'collapsed')!, key: 'ArrowRight' as TreeKey, focus: node('library', 'lib-personal'), expanded: expansionFor({ 'lib-personal': false }) },
    { rule: rules('ArrowRight').find((r) => r.kind === 'library' && r.when === 'expanded')!, key: 'ArrowRight' as TreeKey, focus: node('library', 'lib-personal'), expanded: expansionFor() },
    // ArrowRight: folder collapsed → expand; folder expanded → first child; connection → none.
    { rule: rules('ArrowRight').find((r) => r.kind === 'folder' && r.when === 'collapsed')!, key: 'ArrowRight' as TreeKey, focus: node('folder', 'folder-vms'), expanded: expansionFor() },
    { rule: rules('ArrowRight').find((r) => r.kind === 'folder' && r.when === 'expanded')!, key: 'ArrowRight' as TreeKey, focus: node('folder', 'folder-work'), expanded: expansionFor() },
    { rule: rules('ArrowRight').find((r) => r.kind === 'connection')!, key: 'ArrowRight' as TreeKey, focus: node('connection', 'conn-jump'), expanded: expansionFor() },
    // ArrowLeft: library expanded → collapse; library collapsed → none.
    { rule: rules('ArrowLeft').find((r) => r.kind === 'library' && r.when === 'expanded')!, key: 'ArrowLeft' as TreeKey, focus: node('library', 'lib-team'), expanded: expansionFor() },
    { rule: rules('ArrowLeft').find((r) => r.kind === 'library' && r.when === 'collapsed')!, key: 'ArrowLeft' as TreeKey, focus: node('library', 'lib-team'), expanded: expansionFor({ 'lib-team': false }) },
    // ArrowLeft: folder expanded → collapse; folder collapsed → parent; connection → parent.
    { rule: rules('ArrowLeft').find((r) => r.kind === 'folder' && r.when === 'expanded')!, key: 'ArrowLeft' as TreeKey, focus: node('folder', 'folder-work'), expanded: expansionFor() },
    { rule: rules('ArrowLeft').find((r) => r.kind === 'folder' && r.when === 'collapsed')!, key: 'ArrowLeft' as TreeKey, focus: node('folder', 'folder-vms'), expanded: expansionFor() },
    { rule: rules('ArrowLeft').find((r) => r.kind === 'connection')!, key: 'ArrowLeft' as TreeKey, focus: node('connection', 'conn-jump'), expanded: expansionFor() },
    // Enter per kind.
    { rule: rules('Enter').find((r) => r.kind === 'library')!, key: 'Enter' as TreeKey, focus: node('library', 'lib-team'), expanded: expansionFor() },
    { rule: rules('Enter').find((r) => r.kind === 'folder')!, key: 'Enter' as TreeKey, focus: node('folder', 'folder-prod'), expanded: expansionFor() },
    { rule: rules('Enter').find((r) => r.kind === 'connection')!, key: 'Enter' as TreeKey, focus: node('connection', 'conn-bastion'), expanded: expansionFor() },
    // Shift+F10 per kind.
    { rule: rules('Shift+F10').find((r) => r.kind === 'library')!, key: 'Shift+F10' as TreeKey, focus: node('library', 'lib-team'), expanded: expansionFor() },
    { rule: rules('Shift+F10').find((r) => r.kind === 'folder')!, key: 'Shift+F10' as TreeKey, focus: node('folder', 'folder-prod'), expanded: expansionFor() },
    { rule: rules('Shift+F10').find((r) => r.kind === 'connection')!, key: 'Shift+F10' as TreeKey, focus: node('connection', 'conn-web-prod'), expanded: expansionFor() },
  ];

  it('covers every one of the 28 pinned rules', () => {
    expect(cases).toHaveLength(TREE_KEYBOARD_RULES.length);
    const covered = new Set(cases.map((c) => c.rule));
    for (const rule of TREE_KEYBOARD_RULES) {
      expect(covered.has(rule), `rule ${rule.key}/${rule.kind}/${rule.when ?? '-'}`).toBe(true);
    }
  });

  for (const c of cases) {
    it(`${c.rule.key} on ${c.rule.kind}${c.rule.when ? ` (${c.rule.when})` : ''} → ${c.rule.outcome}`, () => {
      expect(resolveTreeKeyRule(c.key, c.focus.kind, c.focus.kind === 'connection' ? null : c.expanded[c.focus.id] ?? false)).toBe(c.rule);

      const result = applyTreeKey({
        key: c.key,
        focus: c.focus,
        tree: UX_ACCEPTANCE_TREE,
        expanded: c.expanded,
      });

      expect(result.rule).toBe(c.rule);
      expect(result.outcome).toBe(c.rule.outcome);
      expect(result.focusAfter).toBe(c.rule.focusAfter);
    });
  }
});

// ── Edge clamping and expansion side effects (Req 10.13) ─────────────

describe('edge behavior', () => {
  it('clamps ArrowUp on the first visible node and changes nothing else', () => {
    const result = applyTreeKey({
      key: 'ArrowUp',
      focus: { kind: 'library', id: 'lib-personal' },
      tree: UX_ACCEPTANCE_TREE,
      expanded: expansionFor(),
    });
    expect(result.focus).toEqual({ kind: 'library', id: 'lib-personal' });
    expect(result.expansionChange).toBeNull();
    expect(result.action).toBeNull();
  });

  it('clamps ArrowDown on the last visible node', () => {
    const result = applyTreeKey({
      key: 'ArrowDown',
      focus: { kind: 'connection', id: 'conn-quick' },
      tree: UX_ACCEPTANCE_TREE,
      expanded: expansionFor(),
    });
    expect(result.focus).toEqual({ kind: 'connection', id: 'conn-quick' });
  });

  it('ArrowRight on an empty-selection-safe collapsed container only expands it (no focus jump)', () => {
    const result = applyTreeKey({
      key: 'ArrowRight',
      focus: { kind: 'folder', id: 'folder-lab' },
      tree: UX_ACCEPTANCE_TREE,
      expanded: expansionFor({ 'folder-lab': false }),
    });
    expect(result.outcome).toBe('expand-focused-node');
    expect(result.focus).toEqual({ kind: 'folder', id: 'folder-lab' });
    expect(result.expansionChange).toEqual({ nodeId: 'folder-lab', expanded: true });
  });

  it('ArrowLeft collapse of a library keeps focus on the library', () => {
    const result = applyTreeKey({
      key: 'ArrowLeft',
      focus: { kind: 'library', id: 'lib-team' },
      tree: UX_ACCEPTANCE_TREE,
      expanded: expansionFor(),
    });
    expect(result.outcome).toBe('collapse-focused-node');
    expect(result.focus).toEqual({ kind: 'library', id: 'lib-team' });
    expect(result.expansionChange).toEqual({ nodeId: 'lib-team', expanded: false });
  });
});
