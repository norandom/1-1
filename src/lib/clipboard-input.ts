import type { HostProfile } from '../types';
import { isLocalInputTarget } from '../components/session/canvas-keyboard';
import { sendsClipboard } from './clipboard-controller';
export function subscribeClipboardInput(target: Window, documentTarget: Document, direction: HostProfile['clipboardDirection'], active: () => boolean, controller: { toRemote(paste?: boolean): Promise<void> }) {
  const intercepted = new Set<string>();
  const remoteTarget = (target: EventTarget | null) => !isLocalInputTarget(target) && (target as HTMLElement | null)?.tagName === 'CANVAS';
  const down = (event: KeyboardEvent) => {
    const paste = ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && event.code === 'KeyV') || (event.shiftKey && !event.ctrlKey && event.code === 'Insert');
    if (!active() || !remoteTarget(event.target) || !paste || !sendsClipboard(direction)) return;
    event.preventDefault(); event.stopImmediatePropagation();
    intercepted.add(event.code);
    target.dispatchEvent(new Event('rdp-release-keys'));
    if (!event.repeat) void controller.toRemote(true);
  };
  const up = (event: KeyboardEvent) => {
    if (intercepted.delete(event.code)) { event.preventDefault(); event.stopImmediatePropagation(); }
  };
  const paste = (event: ClipboardEvent) => {
    if (!active() || !remoteTarget(event.target) || !sendsClipboard(direction)) return;
    event.preventDefault(); event.stopImmediatePropagation();
    if (intercepted.size) return;
    void controller.toRemote(true);
  };
  target.addEventListener('keydown', down, true);
  target.addEventListener('keyup', up, true);
  documentTarget.addEventListener('paste', paste, true);
  return () => { target.removeEventListener('keydown', down, true); target.removeEventListener('keyup', up, true); documentTarget.removeEventListener('paste', paste, true); };
}
