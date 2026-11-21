import { usePlatformLabel } from '../../hooks/usePlatformLabel';

/** macOS draws the traffic lights centered 15 px below the window's top edge. */
const TITLEBAR_HEIGHT = 30;

export function Titlebar() {
  const { isMac } = usePlatformLabel();

  // Windows/Linux use the native decorated title bar. Only macOS needs
  // content beneath its overlay title bar: a strip the height of a native
  // title bar, so the traffic lights sit centered in it, with the window
  // title in the middle as AppKit would draw it. The brand is in the sidebar.
  if (!isMac) return null;

  return (
    <div
      data-tauri-drag-region
      className="relative flex items-center justify-center shrink-0 select-none bg-[var(--color-surface-0)]"
      style={{ height: TITLEBAR_HEIGHT, borderBottom: '1px solid var(--color-border)' }}
    >
      <span className="pointer-events-none text-[13px] font-semibold text-[var(--color-text-secondary)]">
        1-1
      </span>
    </div>
  );
}
