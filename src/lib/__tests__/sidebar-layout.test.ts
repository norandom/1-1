import { describe, expect, it } from 'vitest';
import {
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  SIDEBAR_RAIL_WIDTH,
  SIDEBAR_STRIP_WIDTH,
  clampSidebarWidth,
  draggedSidebarWidth,
  sidebarOuterWidth,
  sidebarWidthForKey,
} from '../sidebar-layout';

describe('sidebar layout', () => {
  it('clamps stored and dragged widths to the supported range', () => {
    expect(clampSidebarWidth(10)).toBe(SIDEBAR_MIN_WIDTH);
    expect(clampSidebarWidth(9000)).toBe(SIDEBAR_MAX_WIDTH);
    expect(clampSidebarWidth(301.6)).toBe(302);
    expect(clampSidebarWidth(Number.NaN)).toBe(SIDEBAR_DEFAULT_WIDTH);
  });

  it('hidden wins over the rail, and the rail ignores the saved width', () => {
    expect(sidebarOuterWidth({ hidden: true, collapsed: true, width: 400 })).toBe(SIDEBAR_STRIP_WIDTH);
    expect(sidebarOuterWidth({ hidden: false, collapsed: true, width: 400 })).toBe(SIDEBAR_RAIL_WIDTH);
    expect(sidebarOuterWidth({ hidden: false, collapsed: false, width: 400 })).toBe(400);
  });

  it('follows the pointer from where the drag started', () => {
    expect(draggedSidebarWidth(260, 260, 340)).toBe(340);
    expect(draggedSidebarWidth(260, 260, 0)).toBe(SIDEBAR_MIN_WIDTH);
  });

  it('resizes from the keyboard in steps and to either end', () => {
    expect(sidebarWidthForKey(260, 'ArrowRight')).toBe(276);
    expect(sidebarWidthForKey(260, 'ArrowLeft')).toBe(244);
    expect(sidebarWidthForKey(260, 'Home')).toBe(SIDEBAR_MIN_WIDTH);
    expect(sidebarWidthForKey(260, 'End')).toBe(SIDEBAR_MAX_WIDTH);
    expect(sidebarWidthForKey(260, 'Enter')).toBeNull();
  });
});
