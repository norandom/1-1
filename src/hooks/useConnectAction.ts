import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import type { NodeId } from '../types';
import { createConnectAction } from '../lib/connect-action';
import { useLibraryStore } from '../stores/libraryStore';

/**
 * Task 8.5 (Req 10.6, design.md L379): the shared connect entry. The
 * tree (Enter / double-click / Connect menu), the dashboard (double-click
 * / Connect button) and quick connect all go through the exact pure core
 * in `src/lib/connect-action.ts` — select the connection node, then
 * navigate to `/session/:connectionId` (SessionView connects on mount;
 * 7.1 admission makes duplicate connects idempotent).
 */
export function useConnectAction() {
  const navigate = useNavigate();
  const selectNode = useLibraryStore((s) => s.selectNode);

  const connect = useCallback(
    (node: NodeId) => {
      createConnectAction({ selectNode, navigate: (to) => navigate(to) })(node);
    },
    [selectNode, navigate]
  );

  /** Same single action for callers holding a bare connection id. */
  const connectById = useCallback(
    (connectionId: string) => connect({ kind: 'connection', id: connectionId }),
    [connect]
  );

  return { connect, connectById };
}
