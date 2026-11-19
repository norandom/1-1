# Project Structure

## Organization Philosophy

Layer-first by runtime boundary: webview UI (`src/`) vs Rust backend (`src-tauri/src/`), each split by functional area (connections, session, rendering, persistence). Spec-driven: every feature lives under `.kiro/specs/<feature>/`.

## Directory Patterns

### Frontend UI
**Location**: `/src/components/{connections,session,settings,layout,ui}/`
**Purpose**: React components grouped by feature area; `ui/` holds shadcn primitives only.
**Example**: `src/components/layout/Sidebar.tsx`

### Frontend state & logic
**Location**: `/src/stores/`, `/src/hooks/`, `/src/lib/`
**Purpose**: Zustand stores (one per domain), React hooks, and framework-free logic (frame protocol, input mapping, typed Tauri invoke wrappers in `lib/tauri.ts`).
**Example**: `src/lib/frame-protocol.ts` must stay UI-free and unit-testable.

### Rust backend
**Location**: `/src-tauri/src/{rdp,renderer,commands,store,utils}/`
**Purpose**: `rdp/` session actor + IronRDP client; `renderer/` SharedFrame + wgpu; `commands/` Tauri IPC handlers (registered in `lib.rs`); `store/` JSON persistence + keyring; `utils/` probes.
**Example**: new IPC command goes in `commands/<domain>.rs` + registration in `lib.rs`.

### Specs
**Location**: `/​.kiro/specs/<feature>/{requirements,design,tasks}.md`
**Purpose**: Kiro spec documents; phase and approvals tracked in `spec.json`.

## Naming Conventions

- **Files**: React components `PascalCase.tsx`; TS libs `kebab-case.ts`; Rust modules `snake_case.rs`
- **Components**: Named exports, PascalCase (`export function SessionView()`)
- **Functions**: camelCase (TS), snake_case (Rust); Tauri commands snake_case invoked as-is

## Import Organization

```typescript
import { useSessionStore } from '../../stores/sessionStore'; // Relative for app code
import { Button } from '@/components/ui/button';             // @/ alias → src/
```

**Path Aliases**:
- `@/`: `src/`

## Code Organization Principles

- UI components never call `invoke` directly; they go through `src/lib/tauri.ts` wrappers.
- Frame/input logic in `src/lib/` is pure and unit-tested; DOM/canvas work stays in components.
- Rust: one async task per session; IPC handlers stay thin, delegate to `rdp/` and `store/`.
- Types are mirrored, not generated: change `src/types.ts` and `src-tauri/src/store/*.rs` together.

---

_Document patterns, not file trees. New files following patterns shouldn't require updates_
