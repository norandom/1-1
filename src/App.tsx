import { useUpdateStore } from './stores/updateStore';
import { useEffect, useState, useCallback } from 'react';
import { Routes, Route } from 'react-router-dom';
import { AnimatePresence } from 'framer-motion';
import { Titlebar } from './components/layout/Titlebar';
import { Sidebar } from './components/layout/Sidebar';
import { StatusBar } from './components/layout/StatusBar';
// Task 8.5 (Req 11.5): the `/` panel is selection-driven — the dashboard
// of the selected node or the start view (quick connect + shortcuts).
import { MainPanel } from './components/dashboard/MainPanel';
import { ConnectionForm } from './components/connections/ConnectionForm';
import { QuickConnect } from './components/connections/QuickConnect';
import { SessionView } from './components/session/SessionView';
// Task 8.6 (Req 11.1): the persistent document tab strip, hoisted above
// the routes so open sessions stay visible across navigation.
import { DocumentTabs } from './components/session/DocumentTabs';
// Task 8.7 (Req 10.12): the persistent recovery instructions view —
// hoisted like the tab strip so it stays visible across navigation for
// as long as the store is write-blocked (it cannot be dismissed).
import { RecoveryBanner } from './components/layout/RecoveryBanner';
import { SettingsPage } from './components/settings/SettingsPage';
import { ErrorBoundary } from './components/ErrorBoundary';
import { ToastContainer } from './components/Toast';
import { ConnectPasswordDialog } from './components/connections/ConnectPasswordDialog';
import { ShortcutOverlay } from './components/ShortcutOverlay';
import { useTheme } from './hooks/useTheme';
import { useKeyboard } from './hooks/useKeyboard';
import { useSettingsStore } from './stores/settingsStore';
import { useConnectionStore } from './stores/connectionStore';
import { useLibraryStore, selectIsWriteBlocked } from './stores/libraryStore';
import { useToastStore } from './stores/toastStore';
import { RECOVERY_BLOCKED_MESSAGE } from './lib/recovery';
import { TooltipProvider } from '@/components/ui/tooltip';

export default function App() {
  const fetchSettings = useSettingsStore((s) => s.fetchSettings);
  const fetchConnections = useConnectionStore((s) => s.fetchConnections);
  // Task 8.2 (Req 10.1/10.9): the sidebar tree state — also restores the
  // persisted selection/expansion and aggregates real tags for filtering.
  const fetchTree = useLibraryStore((s) => s.fetchTree);
  // Task 8.7 (Req 10.12): the backend store mode feeds the recovery
  // banner and the mutation gating; the only exit is a valid reload.
  const refreshStatus = useLibraryStore((s) => s.refreshStatus);
  const mutationsBlocked = useLibraryStore(selectIsWriteBlocked);
  const addToast = useToastStore((s) => s.addToast);
  const [showNewConnection, setShowNewConnection] = useState(false);
  const [showShortcuts, setShowShortcuts] = useState(false);

  useEffect(() => { void useUpdateStore.getState().check(true); }, []);

  // Initialize theme
  useTheme();

  // Initialize data
  useEffect(() => {
    fetchSettings();
    fetchConnections();
    fetchTree();
    refreshStatus();
  }, [fetchSettings, fetchConnections, fetchTree, refreshStatus]);

  // Keyboard shortcuts
  const handleNewConnection = useCallback(() => {
    // Task 8.7 (Req 10.12): no new mutation affordances while blocked.
    if (mutationsBlocked) {
      addToast({ message: RECOVERY_BLOCKED_MESSAGE, type: 'error' });
      return;
    }
    setShowNewConnection(true);
  }, [mutationsBlocked, addToast]);

  const handleShowShortcuts = useCallback(() => {
    setShowShortcuts((prev) => !prev);
  }, []);

  useKeyboard({ onNewConnection: handleNewConnection, onShowShortcuts: handleShowShortcuts });

  return (
    <TooltipProvider>
      <div className="flex flex-col h-screen w-screen overflow-hidden bg-[var(--color-surface-0)]">
        {/* Titlebar */}
        <Titlebar />

        {/* Main content area */}
        <div className="flex flex-1 min-h-0">
          {/* Sidebar */}
          <Sidebar />

          {/* Page content */}
          <main
            id="main-panel"
            tabIndex={-1}
            className="flex flex-1 min-w-0 flex-col overflow-hidden focus:outline-none"
          >
            {/* Task 8.7 (Req 10.12): the recovery instructions view sits
                above everything else in the panel — persistent across
                navigation and not dismissible while the store is
                write-blocked. */}
            <RecoveryBanner />
            {/* Task 8.6 (Req 11.1): tabs live above the routes — visible
                on session views, dashboards and the start view alike. */}
            <DocumentTabs />
            <div className="relative min-h-0 flex-1 overflow-hidden">
              <ErrorBoundary>
                <Routes>
                  <Route path="/" element={<MainPanel />} />
                  <Route path="/session/:id" element={<SessionView />} />
                  <Route path="/settings" element={<SettingsPage />} />
                </Routes>
              </ErrorBoundary>
            </div>
          </main>
        </div>

        {/* Status Bar */}
        <StatusBar />

        {/* Toast Notifications */}
        <ToastContainer />
        <ConnectPasswordDialog />

        {/* Quick Connect Modal */}
        <QuickConnect />

        {/* New Connection Modal (from Cmd/Ctrl+N) */}
        <AnimatePresence>
          {showNewConnection && (
            <ConnectionForm
              connection={null}
              onClose={() => setShowNewConnection(false)}
            />
          )}
        </AnimatePresence>

        {/* Shortcut Overlay (Cmd/Ctrl+?) */}
        <AnimatePresence>
          {showShortcuts && (
            <ShortcutOverlay onClose={() => setShowShortcuts(false)} />
          )}
        </AnimatePresence>
      </div>
    </TooltipProvider>
  );
}
