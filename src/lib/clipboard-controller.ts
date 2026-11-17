import type { HostProfile } from '../types';

export type ClipboardState = { kind: 'idle' | 'busy' | 'accepted' | 'copied' | 'error'; message: string };
type Dependencies = {
  direction: HostProfile['clipboardDirection'];
  read(): Promise<string>;
  write(text: string): Promise<void>;
  publish(text: string): Promise<void>;
  request(): Promise<void>;
  paste(): Promise<void>;
  allowIncoming?(): boolean;
  state(state: ClipboardState): void;
};
export const sendsClipboard = (direction: HostProfile['clipboardDirection']) => direction === 'bidirectional' || direction === 'localToRemote';
export const receivesClipboard = (direction: HostProfile['clipboardDirection']) => direction === 'bidirectional' || direction === 'remoteToLocal';

/** One owner per active session. No clipboard payload is persisted or logged. */
export function createClipboardController(deps: Dependencies) {
  let disposed = false;
  let busy = false;
  let sending = false;
  let receiving = false;
  let revision = 0;
  let received: (() => void) | undefined;
  let receiveFailed: (() => void) | undefined;
  let writeTail = Promise.resolve();
  const report = (kind: ClipboardState['kind'], message: string) => { if (!disposed) deps.state({ kind, message }); };
  async function bounded<T>(work: Promise<T>): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([work, new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('Clipboard operation timed out. Retry resync; reconnect if the remote channel remains unresponsive.')), 10000); })]);
    } finally { clearTimeout(timer); }
  }
  return {
    async toRemote(paste = false) {
      if (disposed || busy) return;
      if (!sendsClipboard(deps.direction)) { report('error', 'Sending clipboard is disabled for this session.'); return; }
      busy = true; sending = true; const current = ++revision;
      report('busy', 'Reading local clipboard…');
      try {
        const text = await bounded(deps.read());
        if (disposed || current !== revision) return;
        if (new TextEncoder().encode(text).length > 1024 * 1024) throw new Error('Clipboard text exceeds the 1 MiB limit.');
        report('busy', 'Waiting for remote clipboard acknowledgement…');
        await bounded(deps.publish(text));
        if (disposed || current !== revision) return;
        report('accepted', 'Remote accepted clipboard formats.');
        if (paste) await deps.paste();
      } catch (error) { if (current === revision) report('error', error instanceof Error ? error.message : typeof error === 'string' ? error : 'Clipboard transfer failed. Retry resync.'); }
      finally { if (current === revision) { busy = false; sending = false; } }
    },
    async fromRemote() {
      if (disposed || busy) return;
      if (!receivesClipboard(deps.direction)) { report('error', 'Receiving clipboard is disabled for this session.'); return; }
      busy = true; receiving = true;
      report('busy', 'Requesting remote clipboard…');
      const applied = new Promise<void>((resolve, reject) => { received = resolve; receiveFailed = () => reject(new Error('Cannot write the local clipboard. Focus 1-1 and retry resync from remote.')); });
      try { await bounded(Promise.all([deps.request(), applied])); }
      catch (error) { report('error', error instanceof Error ? error.message : typeof error === 'string' ? error : 'Remote clipboard request failed.'); }
      finally { busy = false; receiving = false; received = undefined; receiveFailed = undefined; }
    },
    async receive(text: string) {
      if (disposed || !receivesClipboard(deps.direction)) return;
      if (!receiving && deps.allowIncoming && !deps.allowIncoming()) return;
      // A remote event during local publication must not overwrite what is being sent.
      if (sending) return;
      const current = ++revision;
      try {
        if (new TextEncoder().encode(text).length > 1024 * 1024) throw new Error('Remote clipboard text exceeds the 1 MiB limit.');
        const write = writeTail.then(async () => { if (!disposed && current === revision) await deps.write(text); });
        writeTail = write.catch(() => {});
        await bounded(write);
        if (!disposed && current === revision) { report('copied', 'Remote text copied to local clipboard.'); received?.(); }
      } catch { if (current === revision) { receiveFailed?.(); report('error', 'Cannot write the local clipboard. Focus 1-1 and retry resync from remote.'); } }
    },
    fail(message: string) { report('error', message); },
    dispose() { disposed = true; revision++; },
  };
}
export type ClipboardController = ReturnType<typeof createClipboardController>;
