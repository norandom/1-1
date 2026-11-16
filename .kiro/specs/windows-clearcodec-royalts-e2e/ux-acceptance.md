# UX acceptance — annotated interaction states (task 8.1)

This document pins the expected keyboard, status and focus outcomes of the
Royal TS-style tree/dashboard/tab UI **before any component work starts**, as
mandated by design.md ("UX acceptance matrix"): *"Create ux-acceptance.md with
annotated tree/dashboard, filtered-ancestor, rail and tab/focus states before
UI work."*

It is a contract, not a walkthrough. The machine-checkable form is the fixture
modules; tasks 8.2–8.6 build `ConnectionTree`, `DashboardView`,
`DocumentTabs` and `StartView` against these states, and their tests import
the fixtures so the doc and the code cannot drift apart:

| Fixture module | Pins (exports) | Machine-pinned by |
|---|---|---|
| `src/lib/ux-tree-states.ts` | `UX_ACCEPTANCE_TREE`, `UX_SESSION_STATUSES`, `UX_CONNECTION_TAGS`, `SESSION_STATUS_TEXT`, `TREE_AGGREGATE_STATUS_TEXT`, `TREE_SAVED_EXPANSION_DEFAULT`, `TREE_A11Y_STATE`, `TREE_KEYBOARD_RULES`, `TREE_KEYBOARD_SCENARIOS`, `MOVE_STATE_PINS`, `FILTER_STATE_PINS`, `RAIL_STATE_PINS` | `src/lib/__tests__/ux-tree-states.test.ts` |
| `src/lib/ux-tab-states.ts` | `TAB_LIST_STATE_PINS`, `DASHBOARD_WITH_TABS_PINS`, `CLOSE_TAB_PINS` | `src/lib/__tests__/ux-tab-states.test.ts` |

Shared vocabulary added to `src/types.ts` (pure types, no runtime change):
`NodeId` (`{ kind: 'library' | 'folder' | 'connection'; id: string }`, the
design.md `LibraryStore` contract) and `MainPanelView`.

## Scope and exclusions (design.md, verbatim mandate)

This remains a scoped Royal TS-inspired workflow. **Excluded**: credential
documents, per-library files, document open/close semantics and docking.
Fixtures deliberately use the existing shapes (`LibraryTree`,
`ConnectionSummary`, `SessionStatus`) — no new component system, no UI
framework, no storybook; plain typed fixture data + Vitest.

## Annotated fixture tree (shared by all states)

Two libraries; nested folders; one library-root connection (the 10.10
migration shape); every live status plus "no session yet". Saved expansion
(`TREE_SAVED_EXPANSION_DEFAULT`): Personal Library, Work, Lab, Team Library
and Production expanded; **VMs collapsed**. Selected node: `bastion`.

```
Personal Library                 L1  ▾  1 of 4 connected
  Work                           L2  ▾  1 of 3 connected
    VMs                          L3  ▸  1 of 2 connected
      web-dev-01   10.0.0.11  [web,dev]      (hidden — VMs collapsed)  Connected
      db-dev-02    10.0.0.12  [db,dev]       (hidden — VMs collapsed)  Connecting…
    term-box      10.0.0.13  [ssh,dev]                                  Disconnected (no session)
  Lab                            L2  ▾  0 of 1 connected
    jump-host     10.0.0.14  [lab,jump]                                 Reconnecting…
Team Library                     L1  ▾  1 of 4 connected
  Production                     L2  ▾  1 of 3 connected
    web-prod      10.0.1.21  [web,prod]                                 Error
    db-prod       10.0.1.22  [db,prod]       stopped session             Disconnected
    bastion       10.0.1.23  [prod]          ← selected                 Connected
  quick-lab       10.0.1.24  [lab]           library root (10.10)       Disconnected (no session)
```

Status is **text, never color-only** (10.4/10.14). Connection status text is
`SESSION_STATUS_TEXT` (Disconnected / Connecting… / Connected / Reconnecting…
/ Error); a connection without a session reads `Disconnected` (design.md: tree
status is that single session's status, or disconnected if none exists).
Library/folder nodes expose the descendant aggregate `<connected> of <total>
connected`.

## Row 1 — Keyboard tree (10.13, 10.14; also 10.3, 10.4, 10.5, 10.6, 10.7)

Roving focus: exactly one tree node is in the tab order; arrow/Home/End keys
move it. Every node exposes name, level, kind, expansion, selection and
textual status to accessibility (10.14). Decision table (`TREE_KEYBOARD_RULES`;
container keys split by expansion state):

| Key | Library/Folder collapsed | Library/Folder expanded | Connection |
|---|---|---|---|
| ArrowUp / ArrowDown | focus previous / next **visible** node (clamped at the edges, skip collapsed subtrees) | same | same |
| ArrowRight | expand, keep focus | focus first child | no-op |
| ArrowLeft | library: no-op · folder: focus parent | collapse, keep focus | focus parent |
| Home / End | first / last visible node | same | same |
| Enter | select its **dashboard** (10.5), tree focus kept | same | **connect** (10.6), focus moves to the opened session tab |
| Shift+F10 | open context menu (focus into menu, visible; returns to the node) | same | same |

Pinned concrete transitions (`TREE_KEYBOARD_SCENARIOS`, 22 rows) include:
ArrowDown from `term-box` skips VMs' hidden children to `Lab`; ArrowDown on
`quick-lab` (last) and ArrowUp on `Personal Library` (first) clamp; ArrowRight
expands `VMs` in place; ArrowLeft on `VMs` (collapsed) → `Work`, on
`jump-host` → `Lab`, on `quick-lab` → `Team Library`; Enter on `Production`
selects its dashboard, on `bastion`/`quick-lab` connects. Shift+F10 menus
(`expectContextMenuItems`) are exactly the 10.7 sets plus the keyboard Move
entry (10.14) for movable kinds — connections: Connect, Edit, Duplicate,
Move…, Delete; folders: New Connection, New Folder, Rename, Move…, Delete;
libraries (not movable): New Connection, New Folder, Rename, Delete.

## Row 2 — Move (10.8, 10.14, 10.15)

The keyboard Move dialog and drag-and-drop are two triggers of **one shared
transaction**. Pinned states (`MOVE_STATE_PINS`):

- Success (both triggers pinned): the node lands under the destination and
  focus follows the moved node.
- Validation failure — moving `Work` into its own subtree `VMs`: previous
  persisted tree retained byte-for-byte, visible location restored, focus
  restored to the moved node, message *“Cannot move a folder into its own
  subtree.”*
- Persistence failure — dragging `jump-host` onto `VMs` when the write fails:
  same restore contract, message *“Move failed: the library file could not be
  written.”*

## Row 3 — Filter (10.9)

Matching by name, host or tags shows **matching nodes plus all their
ancestors**, in tree order (`FILTER_STATE_PINS`): query `web` (name) reveals
`web-dev-01` and `web-prod` with five ancestors — including `VMs`, which the
saved state keeps collapsed; `10.0.0.14` (host) shows `jump-host` with
Personal Library/Lab; `ssh` (tags only) shows `term-box` with Personal
Library/Work; `zzz-no-match` shows an empty tree with the text *“No matching
connections.”*

Expansion while filtering is **temporary**: `temporaryExpansion` may open a
saved-collapsed ancestor (VMs during `web`), but `savedExpansionDuringFilter`
must deep-equal `savedExpansionBefore` at every moment, and clearing the
filter restores the saved expansion exactly (`restoredExpansionAfterClear`).
Filter state is never written back to the persisted expansion (10.9).

## Row 4 — Dashboard with tabs (11.6; also 11.1, 11.2, 11.5)

Pinned states: a tab strip with two open sessions (active tab underlined,
hover affordance on the other, textual status per tab); the start view when
nothing is open and nothing is selected; a dashboard showing while a session
tab stays open (no active tab). Sequence pin (`DASHBOARD_WITH_TABS_PINS`):
with `bastion` (Connected) and `jump-host` (Reconnecting…) open, selecting
`Production`, then `Personal Library`, swaps the main panel to each dashboard
while **both tabs remain and both sessions keep their status** (nothing
disconnects, nothing is removed); selecting the `bastion` tab returns the main
panel to exactly that session. Tab identity is per session (11.4): `tabId` =
session id, stable across the whole journey.

## Row 5 — Close tab (11.3, 11.7; also 11.5)

Tabs are named and expose a **named close control** `Close <name>`; keyboard
focus on tabs is visible. Closing disconnects exactly that session and
removes exactly that tab (11.3). Focus after closing is deterministic
(`CLOSE_TAB_PINS`):

1. the **adjacent tab** — the next one if it exists, else the previous one;
2. if no tab remains and a library/folder is selected → its **dashboard**;
3. if no tab remains and nothing is selected → the **start view**.

Focus never lands on the closed tab.

## Row 6 — Status/rail (10.11; also 10.4, 10.14)

Every connection shows its status as **text in addition to any color/dot**.
Collapsing the sidebar is an **explicit, named control** — `Collapse sidebar`
/ `Expand sidebar` — that shrinks it to a rail of library icons, each rail
entry still exposing the library name plus the textual aggregate status
(e.g. *“Personal Library, 1 of 4 connected”*). Collapse **preserves all tree
state** (selection `bastion` and the saved expansion, unchanged through
collapse and re-expand) and the preference persists through the existing
`AppSettings.sidebarCollapsed` setting.

## Requirements covered by this pin

10.3, 10.4, 10.5, 10.6, 10.7, 10.8, 10.9, 10.10 (shape), 10.11, 10.13, 10.14,
10.15, 11.1, 11.2, 11.3, 11.4, 11.5, 11.6, 11.7 — task 8.1's primary gates are
**10.13, 10.14, 11.6, 11.7**; the others are annotated because the same
states pin them for tasks 8.2–8.6.
