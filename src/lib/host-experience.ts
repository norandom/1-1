import type { HostProfile } from '../types';
export const CLIPBOARD_OPTIONS: readonly [HostProfile['clipboardDirection'], string][] = [
  ['bidirectional', 'Both directions'], ['localToRemote', 'Local to remote'], ['remoteToLocal', 'Remote to local'], ['off', 'Off'],
];
export const CLIPBOARD_HELP = 'Clipboard direction applies on reconnect. Resync in the status bar uses the active session policy.';

export const VISUAL_OPTIONS = [
  ['fontSmoothing', 'Font smoothing'],
  ['fullWindowDrag', 'Window contents while dragging'],
  ['wallpaper', 'Desktop wallpaper'],
  ['menuAnimations', 'Menu animations'],
  ['themes', 'Visual themes'],
  ['cursorShadow', 'Cursor shadow'],
  ['cursorBlinking', 'Cursor blinking'],
  ['desktopComposition', 'Desktop composition (legacy)'],
] as const;
export const NETWORK_OPTIONS: readonly [HostProfile['networkHint'], string][] = [
  ['lan', 'LAN'], ['wan', 'WAN'], ['broadbandHigh', 'High-speed broadband'],
  ['broadbandLow', 'Low-speed broadband'], ['satellite', 'Satellite'], ['modem', 'Modem'],
];
export const SCALING_OPTIONS: readonly [HostProfile['scaling'], string][] = [
  ['automatic', 'Automatic (match this display)'], ['custom', 'Custom percentage'],
  ['off', 'Off: 100% (most space)'], ['lowResolution', 'Low resolution (larger, softer)'],
];
export const SCALE_PERCENT_OPTIONS: readonly number[] = [100, 125, 150, 175, 200, 225, 250, 300];
export const SCALING_HELP = 'Automatic scales the remote desktop to this display, so text keeps its size on Retina and HiDPI screens. Custom uses the percentage you choose. Off keeps 100%, with the most space and small text on those screens. Low resolution sends fewer pixels and enlarges them locally. Applies when the remote resizes with the window, otherwise on the next connection.';
export const TRANSFER_PACE_OPTIONS: readonly [HostProfile['transferPace'], string][] = [
  ['responsive', 'Keep the session responsive (default)'], ['balanced', 'Balanced'], ['fastest', 'Fastest copies'],
];
export const TRANSFER_PACE_HELP = 'How files copied from a shared folder share the connection with your input and the screen. Responsive sends file data in small pieces, so the pointer and typing stay smooth and copies take longer. Fastest sends each piece whole; the session can stutter during a copy. Applies on the next connection.';
export const EXPERIENCE_HELP = 'Visual effects and connection hints apply on the next connection. Server policy can override these requests.';
export const COMPOSITION_HELP = 'Desktop composition is a legacy, server-dependent effect requiring 32-bit color.';
export const NETWORK_HELP = 'Connection type is a performance hint, not a bandwidth limit or automatic detection.';
export const COLOR_DEPTH_OPTIONS: readonly [HostProfile['colorDepth'], string][] = [
  ['32', '32-bit (default)'], ['24', '24-bit'], ['16', '16-bit'], ['15', '15-bit'],
];
export const COMPRESSION_OPTIONS: readonly [HostProfile['bulkCompression'], string][] = [
  ['off', 'Off (default)'], ['rdp4', 'RDP 4 — MPPC 8 KB'], ['rdp5', 'RDP 5 — MPPC 64 KB'],
  ['rdp6', 'RDP 6 — NCRUSH'], ['rdp61', 'RDP 6.1 — XCRUSH'],
];
export const COLOR_DEPTH_HELP = 'Requests color depth for standard bitmap updates. The graphics pipeline uses its own formats; the server may override this request. Reconnect to apply.';
export const COMPRESSION_HELP = 'Requests RDP bulk compression, independently of graphics codecs. Off does not disable bitmap or graphics compression. Reconnect to apply.';
export type ExperiencePreset = 'low' | 'balanced' | 'quality';
export const PRESETS: readonly [ExperiencePreset, string][] = [['low', 'Low bandwidth'], ['balanced', 'Balanced'], ['quality', 'Full visual quality']];

/** Presets change visual requests only; display behavior and network hint stay explicit. */
export function experiencePreset(preset: ExperiencePreset) {
  const quality = preset === 'quality';
  const low = preset === 'low';
  return {
    fontSmoothing: !low,
    hostProfile: {
      wallpaper: quality, fullWindowDrag: !low, menuAnimations: quality,
      themes: quality, cursorShadow: quality, cursorBlinking: !low,
      desktopComposition: quality,
    },
  };
}
export function presetName(profile: HostProfile, fontSmoothing: boolean): ExperiencePreset | '' {
  return PRESETS.find(([key]) => {
    const preset = experiencePreset(key);
    return preset.fontSmoothing === fontSmoothing && Object.entries(preset.hostProfile)
      .every(([field, value]) => profile[field as keyof HostProfile] === value);
  })?.[0] ?? '';
}
