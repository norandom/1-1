/**
 * The WebView's own context menu (Back, Reload, "More tools > Share" with a
 * tauri.localhost link, Inspect) does not belong in a desktop app. Keep it
 * only where it helps: in text fields, for cut, copy and paste, and over
 * selected text, for copy. 1-1's own menus handle their right-clicks first.
 */
export function wantsBrowserMenu(target: EventTarget | null, selection: string): boolean {
  if (selection.trim() !== '') return true;
  const element = target as { closest?: (selector: string) => unknown } | null;
  if (typeof element?.closest !== 'function') return false;
  return element.closest('input, textarea, [contenteditable=""], [contenteditable="true"]') != null;
}

export function suppressBrowserMenu(target: Document): () => void {
  const onContextMenu = (event: MouseEvent) => {
    const selection = target.getSelection()?.toString() ?? '';
    if (!wantsBrowserMenu(event.target, selection)) event.preventDefault();
  };
  target.addEventListener('contextmenu', onContextMenu);
  return () => target.removeEventListener('contextmenu', onContextMenu);
}
