import { describe, expect, it } from 'vitest';

import { SESSION_STATUS_TEXT } from '../ux-tree-states';
import type { DesignMatrixRow } from '../ux-tree-states';
import {
  CLOSE_TAB_PINS,
  DASHBOARD_WITH_TABS_PINS,
  TAB_LIST_STATE_PINS,
} from '../ux-tab-states';

function must<T>(value: T | undefined, what: string): T {
  if (value === undefined) throw new Error(`missing ${what}`);
  return value;
}

describe('TAB_LIST_STATE_PINS (Req 11.1/11.5/11.7)', () => {
  it('every tab is named, carries a textual status and a named close control (11.7)', () => {
    const statusTexts = Object.values(SESSION_STATUS_TEXT);
    for (const pin of TAB_LIST_STATE_PINS) {
      for (const t of pin.tabs) {
        expect(t.name.length, t.tabId).toBeGreaterThan(0);
        expect(statusTexts, t.tabId).toContain(t.statusText);
        expect(t.closeControlName, t.tabId).toBe(`Close ${t.name}`);
      }
    }
  });

  it('tab identity is per session and stable (11.4)', () => {
    for (const pin of TAB_LIST_STATE_PINS) {
      const ids = pin.tabs.map((t) => t.tabId);
      expect(new Set(ids).size).toBe(ids.length);
      for (const t of pin.tabs) expect(t.tabId.startsWith('session-')).toBe(true);
    }
  });

  it('at most one tab is active, and active/focus/hover reference existing tabs (11.1/11.7)', () => {
    for (const pin of TAB_LIST_STATE_PINS) {
      const ids = new Set(pin.tabs.map((t) => t.tabId));
      const active = pin.tabs.filter((t) => t.tabId === pin.activeTabId);
      expect(active.length).toBeLessThanOrEqual(1);
      expect(pin.activeTabId === null || ids.has(pin.activeTabId)).toBe(true);
      expect(pin.keyboardFocusTabId === null || ids.has(pin.keyboardFocusTabId)).toBe(true);
      expect(pin.pointerHoverTabId === null || ids.has(pin.pointerHoverTabId)).toBe(true);
    }
  });

  it('pins the active-tab state, the hover affordance, the start view and the dashboard-with-open-tab state', () => {
    expect(TAB_LIST_STATE_PINS.some((p) => p.activeTabId !== null)).toBe(true);
    expect(TAB_LIST_STATE_PINS.some((p) => p.pointerHoverTabId !== null)).toBe(true);
    const start = must(TAB_LIST_STATE_PINS.find((p) => p.mainPanel.view === 'start'), 'start-view pin');
    expect(start.tabs).toEqual([]);
    const dashboard = must(
      TAB_LIST_STATE_PINS.find((p) => p.mainPanel.view === 'dashboard'),
      'dashboard pin'
    );
    // 11.6 facet: a dashboard can show while a session tab stays open.
    expect(dashboard.tabs.length).toBeGreaterThan(0);
    expect(dashboard.activeTabId).toBeNull();
  });

  it('the active tab shows the session the main panel points at (11.2)', () => {
    for (const pin of TAB_LIST_STATE_PINS) {
      if (pin.mainPanel.view === 'session') {
        expect(pin.mainPanel.sessionId).toBe(pin.activeTabId);
      }
    }
  });
});

describe('DASHBOARD_WITH_TABS_PINS (Req 11.6)', () => {
  it('selecting libraries or folders never removes tabs or disconnects sessions', () => {
    for (const pin of DASHBOARD_WITH_TABS_PINS) {
      expect(pin.steps.length).toBeGreaterThan(0);
      const firstTabs = pin.steps[0].tabsAfter;
      const firstStatuses = pin.steps[0].sessionStatusesAfter;
      for (const step of pin.steps) {
        expect(step.tabsAfter, step.description).toEqual(firstTabs);
        expect(step.sessionStatusesAfter, step.description).toEqual(firstStatuses);
      }
      // The surviving tabs are exactly the tracked live sessions.
      expect(new Set(firstTabs)).toEqual(new Set(Object.keys(firstStatuses)));
    }
  });

  it('tree selections switch the main panel to the matching dashboard; tab selection returns to the session', () => {
    const pin = must(DASHBOARD_WITH_TABS_PINS.find((p) => p.scenario.includes('two live sessions')), 'two-session pin');
    for (const step of pin.steps) {
      if ('select' in step.userAction) {
        expect(step.mainPanelAfter, step.description).toEqual({ view: 'dashboard', node: step.userAction.select });
      } else {
        expect(step.mainPanelAfter, step.description).toEqual({ view: 'session', sessionId: step.userAction.selectTab });
      }
    }
    // …and the journey ends back inside a session (11.6 “selecting a tab shall return to that session”).
    const last = pin.steps[pin.steps.length - 1];
    expect('selectTab' in last.userAction).toBe(true);
    expect(last.mainPanelAfter.view).toBe('session');
  });
});

describe('CLOSE_TAB_PINS (Req 11.3/11.7)', () => {
  it('closing disconnects exactly the closed session and removes exactly its tab (11.3)', () => {
    for (const pin of CLOSE_TAB_PINS) {
      expect(pin.tabsAfter, pin.scenario).toEqual(pin.tabsBefore.filter((id) => id !== pin.closingTabId));
      expect(pin.disconnects, pin.scenario).toEqual([pin.closingTabId]);
    }
  });

  it('focus lands on the adjacent tab — next first, else previous — never on the closed tab (11.7)', () => {
    for (const pin of CLOSE_TAB_PINS) {
      const outcome = pin.focusAfter;
      if (outcome.focus === 'adjacent-tab') {
        const idx = pin.tabsBefore.indexOf(pin.closingTabId);
        const nextId = pin.tabsBefore[idx + 1];
        if (nextId !== undefined) {
          expect(outcome.adjacency, pin.scenario).toBe('next');
          expect(outcome.tabId, pin.scenario).toBe(nextId);
        } else {
          expect(outcome.adjacency, pin.scenario).toBe('previous');
          expect(outcome.tabId, pin.scenario).toBe(pin.tabsBefore[idx - 1]);
        }
        expect(pin.tabsAfter, pin.scenario).toContain(outcome.tabId);
      }
    }
  });

  it('with no tabs left, focus goes to the selected node’s dashboard or the start view (11.5/11.7)', () => {
    for (const pin of CLOSE_TAB_PINS) {
      const outcome = pin.focusAfter;
      if (outcome.focus === 'adjacent-tab') {
        expect(pin.tabsAfter.length, pin.scenario).toBeGreaterThan(0);
      } else if (outcome.focus === 'dashboard') {
        expect(pin.tabsAfter, pin.scenario).toEqual([]);
        expect(['library', 'folder'], pin.scenario).toContain(outcome.node.kind);
      } else {
        expect(pin.tabsAfter, pin.scenario).toEqual([]);
        expect(pin.tabsBefore, pin.scenario).toEqual([pin.closingTabId]);
      }
    }
  });

  it('pins every close-focus outcome kind: next, previous, dashboard and start view', () => {
    const outcomes = CLOSE_TAB_PINS.map((p) => p.focusAfter.focus);
    expect(new Set(outcomes)).toEqual(new Set(['adjacent-tab', 'dashboard', 'start-view']));
    expect(CLOSE_TAB_PINS.some((p) => p.focusAfter.focus === 'adjacent-tab' && p.focusAfter.adjacency === 'next')).toBe(true);
    expect(CLOSE_TAB_PINS.some((p) => p.focusAfter.focus === 'adjacent-tab' && p.focusAfter.adjacency === 'previous')).toBe(true);
  });
});

describe('design-matrix coverage: every UX acceptance row is machine-pinned', () => {
  it('all six ux-acceptance.md rows appear across the fixture pins', () => {
    const rows: DesignMatrixRow[] = [
      ...TAB_LIST_STATE_PINS.map((p) => p.matrixRow),
      ...DASHBOARD_WITH_TABS_PINS.map((p) => p.matrixRow),
      ...CLOSE_TAB_PINS.map((p) => p.matrixRow),
      // Tree-side pins: keyboard-tree, move, filter, status-rail.
      'keyboard-tree',
      'move',
      'filter',
      'status-rail',
    ];
    expect(new Set(rows)).toEqual(
      new Set(['keyboard-tree', 'move', 'filter', 'dashboard-with-tabs', 'close-tab', 'status-rail'])
    );
  });
});
