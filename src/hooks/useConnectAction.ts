import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import type { NodeId } from '../types';
import { createConnectAction } from '../lib/connect-action';
import { sessionForConnection } from '../lib/session-route';
import { useLibraryStore } from '../stores/libraryStore';
import { useSessionStore } from '../stores/sessionStore';

/**
 * Task 8.5/8.6 (Req 10.6, 11.8/11.9, design.md L379/L396): the shared
 * connect entry. The tree (Enter / double-click / Connect menu), the
 * dashboard (double-click / Connect button) and quick connect all go
 * through the exact pure core in `src/lib/connect-action.ts` — select
 * the connection node, then navigate to the existing session's tab
 * (`/session/:sessionId`, session id from the store; 7.1 admission
 * keeps ids stable on reuse and restart) or, for a connection without
 * a session, the transitional route SessionView swaps for the backend
 * session id.
 */
export function useConnectAction() {
  const navigate = useNavigate();
  const selectNode = useLibraryStore((s) => s.selectNode);
  const sessions = useSessionStore((s) => s.sessions);
  const requestRestart = useSessionStore((s) => s.requestRestart);
  const requestConnect = useSessionStore((s) => s.requestConnect);

  const connect = useCallback(
    (node: NodeId) => {
      createConnectAction({
        selectNode,
        navigate: (to) => navigate(to),
        sessionForConnection: (connectionId) =>
          sessionForConnection(sessions.values(), connectionId),
        requestRestart,
        requestConnect,
      })(node);
    },
    [selectNode, navigate, sessions, requestRestart, requestConnect]
  );

  /** Same single action for callers holding a bare connection id. */
  const connectById = useCallback(
    (connectionId: string) => connect({ kind: 'connection', id: connectionId }),
    [connect]
  );

  return { connect, connectById };
}
