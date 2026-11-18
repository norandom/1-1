/**
 * Sidebar layout: a resizable expanded tree, the collapsed library rail, or
 * hidden behind a thin strip at the window edge that brings it back. All three
 * persist through AppSettings (sidebarWidth, sidebarCollapsed, sidebarHidden).
 */

export const SIDEBAR_MIN_WIDTH = 200;
export const SIDEBAR_MAX_WIDTH = 480;
export const SIDEBAR_DEFAULT_WIDTH = 260;
export const SIDEBAR_RAIL_WIDTH = 72;
/** The visible strip left behind by a hidden sidebar. */
export const SIDEBAR_STRIP_WIDTH = 6;
/** Arrow-key resize step on the separator. */
export const SIDEBAR_KEY_STEP = 16;

export interface SidebarLayout {
  hidden: boolean;
  collapsed: boolean;
  width: number;
}

/** Keep a stored or dragged width inside the supported range. */
export function clampSidebarWidth(width: number): number {
  if (!Number.isFinite(width)) return SIDEBAR_DEFAULT_WIDTH;
  return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, Math.round(width)));
}

/** Width the sidebar occupies in the window for a layout. */
export function sidebarOuterWidth({ hidden, collapsed, width }: SidebarLayout): number {
  if (hidden) return SIDEBAR_STRIP_WIDTH;
  if (collapsed) return SIDEBAR_RAIL_WIDTH;
  return clampSidebarWidth(width);
}

/** Width while dragging the separator from `startWidth` at `startX`. */
export function draggedSidebarWidth(startWidth: number, startX: number, clientX: number): number {
  return clampSidebarWidth(startWidth + clientX - startX);
}

/** Keyboard resizing on the separator; `null` for keys it does not handle. */
export function sidebarWidthForKey(width: number, key: string): number | null {
  switch (key) {
    case 'ArrowLeft':
      return clampSidebarWidth(width - SIDEBAR_KEY_STEP);
    case 'ArrowRight':
      return clampSidebarWidth(width + SIDEBAR_KEY_STEP);
    case 'Home':
      return SIDEBAR_MIN_WIDTH;
    case 'End':
      return SIDEBAR_MAX_WIDTH;
    default:
      return null;
  }
}
