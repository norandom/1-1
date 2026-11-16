import { create } from 'zustand';
import type { SessionInfo } from '../types';

interface SessionState {
  sessions: Map<string, SessionInfo>;
  activeSessionId: string | null;
  showPerformanceHud: boolean;
  showQuickConnect: boolean;
  /**
   * Task 8.6 (Req 11.9): a Connect requested for a connection whose
   * terminal session's tab is already open. The shared connect action
   * sets this; SessionView consumes it to reconnect in the same tab
   * (7.1 keeps the session id — stable identity) and clears it.
   */
  restartRequest: { sessionId: string; nonce: number } | null;
  /**
   * Task 8.6 remediation round 2 (findings 3+4): a Connect gesture for
   * a connection WITHOUT a session (fresh, or a retry after a failed
   * connect). The shared connect action sets this right before
   * navigating to the transitional route; SessionView's connect effect
   * re-evaluates on it — a re-press on the SAME `/session/:connectionId`
   * route must fire a NEW connect — and consumes it (single-shot, the
   * restartRequest nonce pattern).
   */
  connectRequest: { connectionId: string; nonce: number } | null;

  addSession: (session: SessionInfo) => void;
  removeSession: (sessionId: string) => void;
  updateSession: (sessionId: string, info: Partial<SessionInfo>) => void;
  setActive: (sessionId: string | null) => void;
  requestRestart: (sessionId: string) => void;
  clearRestartRequest: () => void;
  requestConnect: (connectionId: string) => void;
  clearConnectRequest: () => void;
  togglePerformanceHud: () => void;
  toggleQuickConnect: () => void;
  setShowQuickConnect: (show: boolean) => void;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  sessions: new Map(),
  activeSessionId: null,
  showPerformanceHud: false,
  showQuickConnect: false,
  restartRequest: null,
  connectRequest: null,

  addSession: (session) => {
    const sessions = new Map(get().sessions);
    sessions.set(session.id, session);
    set({ sessions, activeSessionId: session.id });
  },

  removeSession: (sessionId) => {
    const sessions = new Map(get().sessions);
    sessions.delete(sessionId);
    const activeSessionId =
      get().activeSessionId === sessionId
        ? sessions.size > 0
          ? sessions.keys().next().value ?? null
          : null
        : get().activeSessionId;
    set({ sessions, activeSessionId });
  },

  updateSession: (sessionId, info) => {
    const sessions = new Map(get().sessions);
    const existing = sessions.get(sessionId);
    if (existing) {
      sessions.set(sessionId, { ...existing, ...info });
      set({ sessions });
    }
  },

  setActive: (sessionId) => set({ activeSessionId: sessionId }),

  requestRestart: (sessionId) =>
    set({
      restartRequest: {
        sessionId,
        nonce: (get().restartRequest?.nonce ?? 0) + 1,
      },
    }),

  clearRestartRequest: () => set({ restartRequest: null }),

  requestConnect: (connectionId) =>
    set({
      connectRequest: {
        connectionId,
        nonce: (get().connectRequest?.nonce ?? 0) + 1,
      },
    }),

  clearConnectRequest: () => set({ connectRequest: null }),

  togglePerformanceHud: () =>
    set({ showPerformanceHud: !get().showPerformanceHud }),

  toggleQuickConnect: () =>
    set({ showQuickConnect: !get().showQuickConnect }),

  setShowQuickConnect: (show) => set({ showQuickConnect: show }),
}));
