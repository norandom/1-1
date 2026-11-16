import { describe, expect, it, vi } from 'vitest';

import type { NodeId } from '../../types';
import { connectRoute, createConnectAction } from '../connect-action';

// Task 8.5 (Req 10.6): ONE shared connect action consumed by the tree
// (Enter / double-click / Connect menu) and the dashboard (double-click /
// Connect button). The pure core is pinned here; the React binding
// (`useConnectAction`) is a thin wrapper over exactly this factory.

describe('connectRoute', () => {
  it("keeps today's route contract: /session/:connectionId (SessionView connects on mount)", () => {
    expect(connectRoute('conn-bastion')).toBe('/session/conn-bastion');
    expect(connectRoute('conn-quick')).toBe('/session/conn-quick');
  });
});

describe('createConnectAction', () => {
  const conn = (id: string): NodeId => ({ kind: 'connection', id });

  it('selects the connection node, then navigates to its session route', () => {
    const selectNode = vi.fn();
    const navigate = vi.fn();
    const connect = createConnectAction({ selectNode, navigate });

    connect(conn('conn-bastion'));

    expect(selectNode).toHaveBeenCalledTimes(1);
    expect(selectNode).toHaveBeenCalledWith(conn('conn-bastion'));
    expect(navigate).toHaveBeenCalledTimes(1);
    expect(navigate).toHaveBeenCalledWith('/session/conn-bastion');
    // Order matters: the selection (which scopes the dashboard the user
    // returns to) lands before the navigation.
    expect(selectNode.mock.invocationCallOrder[0]).toBeLessThan(
      navigate.mock.invocationCallOrder[0]
    );
  });

  it('is repeatable: every invocation selects and navigates again', () => {
    const selectNode = vi.fn();
    const navigate = vi.fn();
    const connect = createConnectAction({ selectNode, navigate });

    connect(conn('conn-bastion'));
    connect(conn('conn-jump'));

    expect(selectNode).toHaveBeenCalledWith(conn('conn-bastion'));
    expect(selectNode).toHaveBeenCalledWith(conn('conn-jump'));
    expect(navigate).toHaveBeenNthCalledWith(1, '/session/conn-bastion');
    expect(navigate).toHaveBeenNthCalledWith(2, '/session/conn-jump');
  });

  it('does not touch sessions: connecting must never disconnect/remove (11.6)', () => {
    const selectNode = vi.fn();
    const navigate = vi.fn();
    const connect = createConnectAction({ selectNode, navigate });

    connect(conn('conn-web-prod'));

    // The action's whole world is selection + navigation; it holds no
    // session references at all (tabs/hoisting are task 8.6).
    expect(selectNode.mock.calls).toHaveLength(1);
    expect(navigate.mock.calls).toHaveLength(1);
  });
});
