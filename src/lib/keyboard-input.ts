/**
 * How key presses reach the remote (spec rdp-keyboard-layout, tasks 4.1 and
 * 5.1). Printable characters go as Unicode, so the remote gets exactly what
 * the local layout produced, whatever its own layout; shortcuts and named
 * keys go as scancodes, as before. With Super as Ctrl, a Super chord reaches
 * the remote as the same chord with Ctrl, and Super tapped alone as the
 * Windows key.
 */

export type KeyMode = 'automatic' | 'physical';

export type KeyAction =
  | { kind: 'scancode'; code: string; isDown: boolean }
  | { kind: 'text'; text: string };

/** What a key press sends: a scancode, text, or nothing (a dead key). */
export type KeyDecision = 'scancode' | 'text' | 'none';

type KeyInput = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'altKey'> & {
  getModifierState?: (key: string) => boolean;
};

const SUPER = new Set(['MetaLeft', 'MetaRight']);

/** One user-perceived character, not a named key such as "Enter". */
function printable(key: string): boolean {
  if (key.length === 0 || key.length > 2) return false;
  const point = key.codePointAt(0) ?? 0;
  if (String.fromCodePoint(point) !== key) return false; // two characters
  return point >= 0x20 && point !== 0x7f && !(point >= 0x80 && point < 0xa0);
}

/** The plain character a letter or digit key carries ("KeyA" → "a"). */
function plainCharacter(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  return null;
}

/** Text or scancode for one key press (requirement 2.1, 2.2). */
export function decideKey(event: KeyInput, mode: KeyMode): KeyDecision {
  if (mode === 'physical') return 'scancode';
  if (event.key === 'Dead') return 'none';
  // Space, the numeric keypad and named keys keep their key semantics.
  if (event.code === 'Space' || event.code.startsWith('Numpad')) return 'scancode';
  if (!printable(event.key)) return 'scancode';
  const altGraph = event.getModifierState?.('AltGraph') ?? false;
  // AltGr produces characters; browsers on Windows also report it as Ctrl+Alt.
  if (altGraph) return 'text';
  if (event.ctrlKey || event.metaKey) return 'scancode';
  if (event.altKey) {
    // Alt with a key's own character is a shortcut (Alt+F); Option on a Mac
    // layout produces another character (Option+L is "@") and that is text.
    const plain = plainCharacter(event.code);
    if (plain !== null && event.key.toLowerCase() === plain) return 'scancode';
  }
  return 'text';
}

export interface KeyTranslatorOptions {
  keys: KeyMode;
  superAsCtrl: boolean;
}

type KeyEventLike = KeyInput & Pick<KeyboardEvent, 'type' | 'repeat' | 'isTrusted'>;

/**
 * Turns the canvas's key events into remote actions and remembers which path
 * each held key took, so its release matches its press (requirement 2.4).
 * Synthetic releases (`isTrusted` false: focus left the session) release what
 * was sent and never tap the Windows key.
 */
export function createKeyTranslator(options: KeyTranslatorOptions, send: (action: KeyAction) => void) {
  // How each held key went down.
  const held = new Map<string, 'scancode' | 'text' | 'none' | 'super'>();
  let superChord = false;
  let ctrlSent = false;

  const keyDown = (event: KeyEventLike) => {
    const { code } = event;
    if (options.superAsCtrl && SUPER.has(code)) {
      if (!held.has(code)) {
        held.set(code, 'super');
        superChord = false;
      }
      return;
    }
    if (options.superAsCtrl && [...held.values()].includes('super')) {
      // A Super chord: the same chord with Ctrl, always as keys.
      if (!ctrlSent) {
        send({ kind: 'scancode', code: 'ControlLeft', isDown: true });
        ctrlSent = true;
      }
      superChord = true;
      held.set(code, 'scancode');
      send({ kind: 'scancode', code, isDown: true });
      return;
    }
    const decision = decideKey(event, options.keys);
    // A repeated key keeps the path of its first press.
    const path = event.repeat && held.has(code) ? held.get(code)! : decision;
    held.set(code, path);
    if (path === 'text') send({ kind: 'text', text: event.key });
    else if (path === 'scancode') send({ kind: 'scancode', code, isDown: true });
  };

  const keyUp = (event: KeyEventLike) => {
    const { code } = event;
    const path = held.get(code);
    held.delete(code);
    if (path === 'super') {
      if (superChord || ctrlSent) {
        if (ctrlSent && ![...held.values()].includes('super')) {
          send({ kind: 'scancode', code: 'ControlLeft', isDown: false });
          ctrlSent = false;
        }
      } else if (event.isTrusted) {
        // Super alone: the Windows key, so the Start menu stays reachable.
        send({ kind: 'scancode', code, isDown: true });
        send({ kind: 'scancode', code, isDown: false });
      }
      if (![...held.values()].includes('super')) superChord = false;
      return;
    }
    if (path === 'text' || path === 'none') return;
    // A scancode key, or a release for a key this translator never saw.
    send({ kind: 'scancode', code, isDown: false });
  };

  return { keyDown, keyUp };
}
