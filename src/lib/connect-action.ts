import type { NodeId } from '../types';

/**
 * Task 8.5 (Req 10.6): the ONE shared connect action — design.md L379
 * ("shared `useConnectAction` hook"). The tree (Enter / double-click /
 * Connect menu) and the dashboard (double-click / Connect button) both
 * consume exactly this core via `src/hooks/useConnectAction.ts`; nothing
 * else may navigate to a session route on its own.
 *
 * Today the action routes to `/session/:connectionId` and SessionView
 * connects on mount (7.1 makes duplicate connects idempotent — an
 * existing session is returned). Task 8.6 hoists persistent tabs and
 * session-id routes; only `connectRoute` changes then.
 */

/** Session route for a connection — the single source of that string. */
export function connectRoute(connectionId: string): string {
  return `/session/${connectionId}`;
}

/** Collaborators the pure action needs (injected; the hook wires real ones). */
export interface ConnectActionDeps {
  selectNode: (node: NodeId) => void;
  navigate: (path: string) => void;
}

export type ConnectAction = (node: NodeId) => void;

/**
 * Connect a connection node: select it (so the dashboard the user returns
 * to is scoped to it) and navigate to its session route. The action never
 * touches sessions — connecting must not disconnect or remove anything
 * (Req 11.6); tab focus is task 8.6.
 */
export function createConnectAction(deps: ConnectActionDeps): ConnectAction {
  return (node: NodeId) => {
    deps.selectNode(node);
    deps.navigate(connectRoute(node.id));
  };
}
