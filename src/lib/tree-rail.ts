import type { LibraryTree, NodeId, SessionStatus } from '../types';
import { aggregateStatusText } from './tree-a11y';

/**
 * Task 8.3 (Req 10.11 + 10.4/10.14): the collapsed-sidebar rail model.
 * The rail shows one entry per library with a textual aggregate status;
 * the explicit named toggle collapses/expands without touching tree
 * state (selection + saved expansion live in `libraryStore` and are
 * never an input here), and the preference persists through
 * `AppSettings.sidebarCollapsed` (RAIL_STATE_PINS).
 */

export interface RailEntry {
  library: NodeId;
  name: string;
  /** Textual aggregate — in addition to any color/dot (10.4/10.14). */
  statusText: string;
  /** Full accessible name of the rail entry. */
  accessibleName: string;
}

/** One rail entry per library, in tree order, with accessible names. */
export function buildRailEntries(
  tree: LibraryTree,
  statuses: Record<string, SessionStatus>
): RailEntry[] {
  return tree.libraries.map((library) => {
    const statusText = aggregateStatusText(tree, library.id, statuses);
    return {
      library: { kind: 'library', id: library.id },
      name: library.name,
      statusText,
      accessibleName: `${library.name}, ${statusText}`,
    };
  });
}

/** The named collapse control (pinned `RAIL_STATE_PINS.toggleControlName`). */
export function railToggleButtonName(collapsed: boolean): 'Collapse sidebar' | 'Expand sidebar' {
  return collapsed ? 'Expand sidebar' : 'Collapse sidebar';
}

/** Flip the persisted `AppSettings.sidebarCollapsed` preference. */
export function nextSidebarCollapsed(collapsed: boolean): boolean {
  return !collapsed;
}
