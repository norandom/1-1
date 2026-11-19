import type { ConnectionConfig, ConnectionInput, HostProfile } from '../types';
import { DEFAULT_HOST_PROFILE, normalizeHostProfile } from './host-profile';

/**
 * A saved host as a command line, and back (`1-1 import --name … --host …`).
 * Every setting is its own flag, named after the setting in kebab case, so the
 * command carries the host as it is. Passwords are never included. The
 * library and the confirmed server fingerprint travel as `--add-to-lib` and
 * `--fingerprint`.
 */

type Kind = 'string' | 'int' | 'bool' | readonly string[];

const PROFILE: Record<keyof HostProfile, Kind> = {
  idlePreview: 'bool',
  clipboardDirection: ['off', 'bidirectional', 'localToRemote', 'remoteToLocal'],
  colorDepth: ['15', '16', '24', '32'],
  bulkCompression: ['off', 'rdp4', 'rdp5', 'rdp6', 'rdp61'],
  displayMode: ['fit', 'native'],
  resizeMode: ['dynamic', 'fixed'],
  wallpaper: 'bool',
  fullWindowDrag: 'bool',
  menuAnimations: 'bool',
  themes: 'bool',
  cursorShadow: 'bool',
  cursorBlinking: 'bool',
  desktopComposition: 'bool',
  networkHint: ['lan', 'wan', 'broadbandHigh', 'broadbandLow', 'satellite', 'modem'],
  bottomBar: ['always', 'withToolbar'],
  scaling: ['automatic', 'off', 'custom', 'lowResolution'],
  scalePercent: 'int',
  savePassword: 'bool',
};

const kebab = (key: string) => key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
const FINGERPRINT = /^SHA256:[A-Za-z0-9+/]{43}$/;

export interface HostCommandOptions {
  library?: string | null;
  fingerprint?: string | null;
  /** Quote for PowerShell on Windows, a POSIX shell elsewhere. */
  windows: boolean;
}

/** A value as one shell word: bare when safe, otherwise single-quoted. */
function quote(value: string, windows: boolean): string {
  if (/^[A-Za-z0-9._:/+=,-]+$/.test(value) && !value.startsWith('-')) return value;
  return windows ? `'${value.replace(/'/g, "''")}'` : `'${value.replace(/'/g, `'\\''`)}'`;
}

/** The command that recreates `host`, ready to paste into a shell. */
export function hostCommand(host: ConnectionConfig, options: HostCommandOptions): string {
  const args: [string, string][] = [
    ['name', host.name],
    ['host', host.host],
    ['port', String(host.port)],
  ];
  if (host.username) args.push(['username', host.username]);
  if (host.domain) args.push(['domain', host.domain]);
  if (host.displayWidth != null && host.displayHeight != null) {
    args.push(['display-width', String(host.displayWidth)], ['display-height', String(host.displayHeight)]);
  }
  args.push(['font-smoothing', String(host.fontSmoothing)]);
  if (host.colorAccent) args.push(['color-accent', host.colorAccent]);
  for (const tag of host.tags) args.push(['tag', tag]);
  const profile = normalizeHostProfile(host.hostProfile);
  for (const key of Object.keys(PROFILE) as (keyof HostProfile)[]) {
    args.push([kebab(key), String(profile[key])]);
  }
  if (options.library) args.push(['add-to-lib', options.library]);
  if (options.fingerprint) args.push(['fingerprint', options.fingerprint]);
  const program = options.windows ? '1-1.exe' : '1-1';
  return [program, 'import', ...args.map(([flag, value]) => `--${flag} ${quote(value, options.windows)}`)].join(' ');
}

export interface HostImport {
  input: ConnectionInput;
  /** The library to add the host to, by name; created when missing. */
  library: string | null;
  /** The server key fingerprint to verify before connecting. */
  fingerprint: string | null;
}

export type ParsedHostCommand = { ok: true; value: HostImport } | { ok: false; error: string };

/**
 * Read `import --flag value …` (the arguments after the program name). Every
 * flag is checked; an unknown one, a bad value or a missing name or host is
 * an error, so a mistyped command never saves a half-configured host.
 */
export function parseHostCommand(args: readonly string[]): ParsedHostCommand {
  if (args[0] !== 'import') return { ok: false, error: 'Expected the import command: 1-1 import --name … --host …' };
  const fail = (error: string): ParsedHostCommand => ({ ok: false, error });
  const profile: Partial<Record<keyof HostProfile, unknown>> = {};
  const profileFlags = new Map((Object.keys(PROFILE) as (keyof HostProfile)[]).map((key) => [kebab(key), key]));
  const host: Record<string, string> = {};
  const tags: string[] = [];
  let library: string | null = null;
  let fingerprint: string | null = null;
  for (let i = 1; i < args.length; i += 2) {
    const flag = args[i];
    const value = args[i + 1];
    if (!flag.startsWith('--')) return fail(`Unexpected argument "${flag}".`);
    if (value === undefined) return fail(`${flag} needs a value.`);
    const name = flag.slice(2);
    const key = profileFlags.get(name);
    if (key) {
      const kind = PROFILE[key];
      if (kind === 'bool') {
        if (!['true', 'false', 'on', 'off'].includes(value)) return fail(`${flag} takes true or false.`);
        profile[key] = value === 'true' || value === 'on';
      } else if (kind === 'int') {
        if (!/^\d+$/.test(value)) return fail(`${flag} takes a whole number.`);
        profile[key] = Number(value);
      } else if (Array.isArray(kind)) {
        if (!kind.includes(value)) return fail(`${flag} takes one of: ${kind.join(', ')}.`);
        profile[key] = value;
      }
      continue;
    }
    switch (name) {
      case 'name': case 'host': case 'port': case 'username': case 'domain':
      case 'display-width': case 'display-height': case 'font-smoothing': case 'color-accent':
        host[name] = value;
        break;
      case 'tag':
        tags.push(value);
        break;
      case 'add-to-lib':
        library = value.trim() || null;
        break;
      case 'fingerprint':
        if (!FINGERPRINT.test(value)) return fail('--fingerprint takes the SHA256:… value from Review server identity.');
        fingerprint = value;
        break;
      default:
        return fail(`Unknown option ${flag}.`);
    }
  }
  if (!host.name?.trim()) return fail('--name is required.');
  if (!host.host?.trim()) return fail('--host is required.');
  const number = (flag: string, fallback: number | null) => {
    const raw = host[flag];
    if (raw === undefined) return fallback;
    return /^\d+$/.test(raw) ? Number(raw) : NaN;
  };
  const port = number('port', 3389);
  if (!port || port > 65535) return fail('--port takes 1 to 65535.');
  const width = number('display-width', null);
  const height = number('display-height', null);
  if (Number.isNaN(width) || Number.isNaN(height) || (width === null) !== (height === null)) {
    return fail('--display-width and --display-height go together, as whole numbers.');
  }
  const smoothing = host['font-smoothing'];
  if (smoothing !== undefined && !['true', 'false', 'on', 'off'].includes(smoothing)) return fail('--font-smoothing takes true or false.');
  return {
    ok: true,
    value: {
      input: {
        name: host.name.trim(),
        host: host.host.trim(),
        port: port as number,
        username: host.username ?? '',
        domain: host.domain ?? '',
        colorAccent: host['color-accent'] ?? null,
        tags,
        displayWidth: width,
        displayHeight: height,
        fontSmoothing: smoothing === undefined ? true : smoothing === 'true' || smoothing === 'on',
        hostProfile: { ...DEFAULT_HOST_PROFILE, ...normalizeHostProfile(profile as Partial<HostProfile>) },
      },
      library,
      fingerprint,
    },
  };
}

/**
 * Split a pasted command into words, for tests and a paste box: single and
 * double quotes, PowerShell's doubled '' and POSIX '\'' are understood.
 */
export function splitCommand(command: string): string[] {
  const words: string[] = [];
  let word = '';
  let quoted = false;
  let inWord = false;
  for (let i = 0; i < command.length; i++) {
    const c = command[i];
    if (quoted) {
      if (c === "'" && command[i + 1] === "'") { word += "'"; i++; continue; }
      if (c === "'") { quoted = false; continue; }
      word += c;
      continue;
    }
    if (c === "'") { quoted = true; inWord = true; continue; }
    if (c === '\\' && command[i + 1] === "'") { word += "'"; i++; inWord = true; continue; }
    if (/\s/.test(c)) {
      if (inWord) words.push(word);
      word = '';
      inWord = false;
      continue;
    }
    word += c;
    inWord = true;
  }
  if (inWord) words.push(word);
  return words;
}
