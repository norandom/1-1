import { describe, expect, it } from 'vitest';

import {
  RAIL_STATE_PINS,
  TREE_SAVED_EXPANSION_DEFAULT,
  UX_ACCEPTANCE_TREE,
  UX_SESSION_STATUSES,
} from '../ux-tree-states';
import { buildRailEntries, nextSidebarCollapsed, railToggleButtonName } from '../tree-rail';

// ── Rail entries (Req 10.11 + 10.4/10.14 textual aggregate) ──────────

describe('buildRailEntries', () => {
  it('reproduces the pinned rail entries with accessible names', () => {
    expect(buildRailEntries(UX_ACCEPTANCE_TREE, UX_SESSION_STATUSES)).toEqual(
      RAIL_STATE_PINS[0].entries
    );
  });

  it('is identical for the collapsed-rail pin (same libraries, same statuses)', () => {
    expect(buildRailEntries(UX_ACCEPTANCE_TREE, UX_SESSION_STATUSES)).toEqual(
      RAIL_STATE_PINS[1].entries
    );
  });

  it('exposes "name, n of m connected" as the accessible name (never color-only)', () => {
    const entries = buildRailEntries(UX_ACCEPTANCE_TREE, UX_SESSION_STATUSES);
    expect(entries[0].accessibleName).toBe('Personal Library, 1 of 4 connected');
    expect(entries[1].accessibleName).toBe('Team Library, 1 of 4 connected');
  });

  it('is pure: repeated calls agree and inputs stay untouched', () => {
    const statusesSnapshot = JSON.stringify(UX_SESSION_STATUSES);
    const first = buildRailEntries(UX_ACCEPTANCE_TREE, UX_SESSION_STATUSES);
    const second = buildRailEntries(UX_ACCEPTANCE_TREE, UX_SESSION_STATUSES);
    expect(first).toEqual(second);
    expect(JSON.stringify(UX_SESSION_STATUSES)).toBe(statusesSnapshot);
  });
});

// ── Named toggle control (Req 10.11, RAIL_STATE_PINS.toggleControlName) ──

describe('railToggleButtonName', () => {
  it('names the control "Collapse sidebar" while expanded (pinned)', () => {
    const expanded = RAIL_STATE_PINS.find((p) => p.scenario === 'expanded-sidebar')!;
    expect(railToggleButtonName(expanded.sidebarCollapsed)).toBe(expanded.toggleControlName);
    expect(railToggleButtonName(false)).toBe('Collapse sidebar');
  });

  it('names the control "Expand sidebar" while collapsed (pinned)', () => {
    const collapsed = RAIL_STATE_PINS.find((p) => p.scenario === 'collapsed-rail')!;
    expect(railToggleButtonName(collapsed.sidebarCollapsed)).toBe(collapsed.toggleControlName);
    expect(railToggleButtonName(true)).toBe('Expand sidebar');
  });
});

describe('nextSidebarCollapsed', () => {
  it('flips the preference through AppSettings.sidebarCollapsed', () => {
    expect(nextSidebarCollapsed(false)).toBe(true);
    expect(nextSidebarCollapsed(true)).toBe(false);
  });
});

// ── Tree state preservation through collapse (Req 10.11) ─────────────

describe('RAIL_STATE_PINS preserved state', () => {
  it('pins selection bastion and the saved expansion for both scenarios', () => {
    for (const pin of RAIL_STATE_PINS) {
      expect(pin.preserved.selectedNodeId).toEqual({ kind: 'connection', id: 'conn-bastion' });
      expect(pin.preserved.expanded).toEqual(TREE_SAVED_EXPANSION_DEFAULT);
    }
  });
});
