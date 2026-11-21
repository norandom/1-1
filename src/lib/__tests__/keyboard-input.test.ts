import { describe, expect, it } from 'vitest';
import { createKeyTranslator, decideKey, type KeyAction } from '../keyboard-input';

type Init = Partial<{
  key: string; code: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean;
  altGraph: boolean; repeat: boolean; isTrusted: boolean; type: string;
}>;

const key = (init: Init) => ({
  key: init.key ?? '', code: init.code ?? '', ctrlKey: !!init.ctrlKey, metaKey: !!init.metaKey,
  altKey: !!init.altKey, repeat: !!init.repeat, isTrusted: init.isTrusted ?? true, type: init.type ?? 'keydown',
  getModifierState: (name: string) => name === 'AltGraph' && !!init.altGraph,
});

describe('text or scancode (spec rdp-keyboard-layout 4.1)', () => {
  it('sends printable characters as text', () => {
    expect(decideKey(key({ key: 'a', code: 'KeyA' }), 'automatic')).toBe('text');
    expect(decideKey(key({ key: 'Z', code: 'KeyY' }), 'automatic')).toBe('text'); // QWERTZ, shifted
    expect(decideKey(key({ key: 'ä', code: 'Quote' }), 'automatic')).toBe('text');
    expect(decideKey(key({ key: '|', code: 'IntlBackslash' }), 'automatic')).toBe('text');
  });

  it('treats AltGr and Option results as text, Alt shortcuts as keys', () => {
    // German PC layout: AltGr+Q is "@"; Windows browsers add Ctrl+Alt.
    expect(decideKey(key({ key: '@', code: 'KeyQ', ctrlKey: true, altKey: true, altGraph: true }), 'automatic')).toBe('text');
    // German Mac layout: Option+L is "@".
    expect(decideKey(key({ key: '@', code: 'KeyL', altKey: true }), 'automatic')).toBe('text');
    // Alt+F opens a menu.
    expect(decideKey(key({ key: 'f', code: 'KeyF', altKey: true }), 'automatic')).toBe('scancode');
  });

  it('keeps shortcuts, named keys, Space and the keypad as keys', () => {
    expect(decideKey(key({ key: 'c', code: 'KeyC', ctrlKey: true }), 'automatic')).toBe('scancode');
    expect(decideKey(key({ key: 'v', code: 'KeyV', metaKey: true }), 'automatic')).toBe('scancode');
    for (const named of ['Enter', 'Escape', 'Tab', 'Backspace', 'ArrowUp', 'F5', 'Shift']) {
      expect(decideKey(key({ key: named, code: named }), 'automatic')).toBe('scancode');
    }
    expect(decideKey(key({ key: ' ', code: 'Space' }), 'automatic')).toBe('scancode');
    expect(decideKey(key({ key: '7', code: 'Numpad7' }), 'automatic')).toBe('scancode');
  });

  it('waits on a dead key; the composed character follows as text', () => {
    expect(decideKey(key({ key: 'Dead', code: 'Equal' }), 'automatic')).toBe('none');
    expect(decideKey(key({ key: 'é', code: 'KeyE' }), 'automatic')).toBe('text');
  });

  it('sends everything as keys in Physical mode', () => {
    expect(decideKey(key({ key: 'a', code: 'KeyA' }), 'physical')).toBe('scancode');
    expect(decideKey(key({ key: 'Dead', code: 'Equal' }), 'physical')).toBe('scancode');
  });
});

function translator(options = { keys: 'automatic' as const, superAsCtrl: true }) {
  const sent: KeyAction[] = [];
  const t = createKeyTranslator(options, (action) => sent.push(action));
  const down = (init: Init) => t.keyDown(key({ ...init, type: 'keydown' }));
  const up = (init: Init) => t.keyUp(key({ ...init, type: 'keyup' }));
  return { sent, down, up };
}

const scan = (code: string, isDown: boolean): KeyAction => ({ kind: 'scancode', code, isDown });

describe('the key translator (spec rdp-keyboard-layout 4.2, 5.1)', () => {
  it('sends text once per press, including repeats, and no release for it', () => {
    const { sent, down, up } = translator();
    down({ key: '@', code: 'KeyL', altKey: true });
    down({ key: '@', code: 'KeyL', altKey: true, repeat: true });
    up({ key: '@', code: 'KeyL' });
    expect(sent).toEqual([{ kind: 'text', text: '@' }, { kind: 'text', text: '@' }]);
  });

  it('pairs scancode presses with releases, and sends nothing for a dead key', () => {
    const { sent, down, up } = translator();
    down({ key: 'Enter', code: 'Enter' });
    up({ key: 'Enter', code: 'Enter' });
    down({ key: 'Dead', code: 'Equal' });
    up({ key: 'Dead', code: 'Equal' });
    expect(sent).toEqual([scan('Enter', true), scan('Enter', false)]);
  });

  it('turns a Super chord into the Ctrl chord', () => {
    const { sent, down, up } = translator();
    down({ key: 'Meta', code: 'MetaLeft', metaKey: true });
    down({ key: 'c', code: 'KeyC', metaKey: true });
    up({ key: 'c', code: 'KeyC', metaKey: true });
    down({ key: 'v', code: 'KeyV', metaKey: true });
    up({ key: 'v', code: 'KeyV', metaKey: true });
    up({ key: 'Meta', code: 'MetaLeft' });
    expect(sent).toEqual([
      scan('ControlLeft', true),
      scan('KeyC', true), scan('KeyC', false),
      scan('KeyV', true), scan('KeyV', false),
      scan('ControlLeft', false),
    ]);
  });

  it('keeps a lone Super tap as the Windows key', () => {
    const { sent, down, up } = translator();
    down({ key: 'Meta', code: 'MetaLeft', metaKey: true });
    down({ key: 'Meta', code: 'MetaLeft', metaKey: true, repeat: true });
    up({ key: 'Meta', code: 'MetaLeft' });
    expect(sent).toEqual([scan('MetaLeft', true), scan('MetaLeft', false)]);
  });

  it('releases a chord when focus leaves, without tapping the Windows key', () => {
    const chord = translator();
    chord.down({ key: 'Meta', code: 'MetaLeft', metaKey: true });
    chord.down({ key: 'z', code: 'KeyZ', metaKey: true });
    chord.up({ key: 'z', code: 'KeyZ', isTrusted: false });
    chord.up({ key: 'Meta', code: 'MetaLeft', isTrusted: false });
    expect(chord.sent).toEqual([scan('ControlLeft', true), scan('KeyZ', true), scan('KeyZ', false), scan('ControlLeft', false)]);

    const alone = translator();
    alone.down({ key: 'Meta', code: 'MetaLeft', metaKey: true });
    alone.up({ key: 'Meta', code: 'MetaLeft', isTrusted: false });
    expect(alone.sent).toEqual([]);
  });

  it('leaves Super as the Windows key when Super as Ctrl is off', () => {
    const { sent, down, up } = translator({ keys: 'automatic', superAsCtrl: false });
    down({ key: 'Meta', code: 'MetaLeft', metaKey: true });
    down({ key: 'e', code: 'KeyE', metaKey: true });
    up({ key: 'e', code: 'KeyE' });
    up({ key: 'Meta', code: 'MetaLeft' });
    expect(sent).toEqual([scan('MetaLeft', true), scan('KeyE', true), scan('KeyE', false), scan('MetaLeft', false)]);
  });
});
