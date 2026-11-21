import { describe, expect, it } from 'vitest';
import type { ConnectionConfig } from '../../types';
import { DEFAULT_HOST_PROFILE } from '../host-profile';
import { hostCommand, parseHostCommand, splitCommand } from '../host-command';

const FINGERPRINT = 'SHA256:' + 'A'.repeat(42) + 'Q';

function host(overrides: Partial<ConnectionConfig> = {}): ConnectionConfig {
  return {
    id: 'h1', name: "Bob's build box", host: '10.0.0.51', port: 3390, username: 'dev', domain: 'LAB',
    colorAccent: '#ff8800', tags: ['agents', 'gpu box'], displayWidth: 2560, displayHeight: 1440,
    fontSmoothing: false, createdAt: '', updatedAt: '',
    hostProfile: {
      ...DEFAULT_HOST_PROFILE, scaling: 'custom', scalePercent: 150, savePassword: false,
      clipboardDirection: 'localToRemote', bulkCompression: 'rdp61', wallpaper: true, bottomBar: 'withToolbar',
      keyboardLayout: 0x407, keyboardKeys: 'physical', superAsCtrl: false,
    },
    ...overrides,
  };
}

/** What import should recreate: the saved fields without identity or timestamps. */
function saved(h: ConnectionConfig) {
  const rest: Record<string, unknown> = { ...h };
  for (const key of ['id', 'createdAt', 'updatedAt']) delete rest[key];
  return rest;
}

describe('host command line', () => {
  for (const windows of [true, false]) {
    it(`round-trips every setting through ${windows ? 'PowerShell' : 'a POSIX shell'} quoting`, () => {
      const original = host();
      const command = hostCommand(original, { windows, library: 'Lab hosts', fingerprint: FINGERPRINT });
      expect(command.startsWith(windows ? '1-1.exe import ' : '1-1 import ')).toBe(true);
      expect(command).not.toContain('--password ');
      const parsed = parseHostCommand(splitCommand(command).slice(1));
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      expect(parsed.value.library).toBe('Lab hosts');
      expect(parsed.value.fingerprint).toBe(FINGERPRINT);
      expect(parsed.value.input).toEqual(saved(original));
    });
  }

  it('writes a fixed keyboard layout as a Windows layout ID and reads the named choices', () => {
    expect(hostCommand(host(), { windows: false })).toContain('--keyboard-layout 00000407 ');
    const parse = (value: string) => parseHostCommand(['import', '--name', 'B', '--host', 'b', '--keyboard-layout', value]);
    const layout = (value: string) => {
      const parsed = parse(value);
      return parsed.ok ? parsed.value.input.hostProfile?.keyboardLayout : parsed.error;
    };
    expect(layout('serverDefault')).toBe('serverDefault');
    expect(layout('00020409')).toBe(0x20409);
    expect(layout('qwerty')).toContain('Windows layout ID');
  });

  it('names every host setting as its own flag', () => {
    const command = hostCommand(host(), { windows: false });
    for (const key of Object.keys(DEFAULT_HOST_PROFILE)) {
      expect(command).toContain(`--${key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)} `);
    }
    expect(command).not.toContain('--fingerprint');
    expect(command).not.toContain('--add-to-lib');
  });

  it('defaults what a short command leaves out', () => {
    const parsed = parseHostCommand(['import', '--name', 'Box', '--host', 'box.lan']);
    expect(parsed).toEqual({
      ok: true,
      value: {
        input: {
          name: 'Box', host: 'box.lan', port: 3389, username: '', domain: '', colorAccent: null, tags: [],
          displayWidth: null, displayHeight: null, fontSmoothing: true, hostProfile: DEFAULT_HOST_PROFILE,
        },
        library: null,
        fingerprint: null,
      },
    });
  });

  it('refuses a command that would save a half-configured host', () => {
    const cases: [string[], RegExp][] = [
      [['connect', '--host', 'x'], /import command/],
      [['import', '--host', 'x'], /--name is required/],
      [['import', '--name', 'x'], /--host is required/],
      [['import', '--name', 'x', '--host', 'y', '--scaling', 'huge'], /--scaling takes one of/],
      [['import', '--name', 'x', '--host', 'y', '--wallpaper', 'maybe'], /true or false/],
      [['import', '--name', 'x', '--host', 'y', '--port', '70000'], /1 to 65535/],
      [['import', '--name', 'x', '--host', 'y', '--display-width', '800'], /go together/],
      [['import', '--name', 'x', '--host', 'y', '--password', 'secret'], /Unknown option --password/],
      [['import', '--name', 'x', '--host', 'y', '--fingerprint', 'abc'], /SHA256/],
      [['import', '--name', 'x', '--host'], /needs a value/],
    ];
    for (const [args, error] of cases) {
      const parsed = parseHostCommand(args);
      expect(parsed.ok, args.join(' ')).toBe(false);
      if (!parsed.ok) expect(parsed.error).toMatch(error);
    }
  });
});
