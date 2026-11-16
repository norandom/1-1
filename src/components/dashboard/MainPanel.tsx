import { mainPanelFor } from '../../lib/dashboard-view';
import { useLibraryStore } from '../../stores/libraryStore';
import { DashboardView } from './DashboardView';
import { StartView } from './StartView';

/**
 * Task 8.5 (Req 11.5/11.6, design.md L394-397): the `/` main panel,
 * driven by `libraryStore` selection — the dashboard of the selected
 * tree node, or the start view when nothing (valid) is selected. Session
 * tree node, or the start view when nothing (valid) is selected. Session
 * views come from the `/session/:sessionId` route, not from here;
 * switching selection swaps this panel without touching sessions
 * (DASHBOARD_WITH_TABS_PINS; the tab strip itself is task 8.6).
 */
export function MainPanel() {
  const tree = useLibraryStore((s) => s.tree);
  const selectedNodeId = useLibraryStore((s) => s.selectedNodeId);

  const panel = mainPanelFor(tree, selectedNodeId);
  if (panel.view === 'dashboard') {
    // Keyed by node: a selection change remounts the dashboard so its
    // descendants fetch starts clean for the new node.
    return <DashboardView key={`${panel.node.kind}:${panel.node.id}`} node={panel.node} />;
  }
  return <StartView />;
}
