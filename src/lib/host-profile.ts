import type { ConnectionConfig, HostProfile } from '../types';

export const DEFAULT_HOST_PROFILE: Readonly<HostProfile> = Object.freeze({
  clipboardDirection: 'bidirectional',
  displayMode: 'fit', resizeMode: 'dynamic', wallpaper: false,
  fullWindowDrag: true, menuAnimations: false, themes: false,
  cursorShadow: false, cursorBlinking: true, desktopComposition: false,
  networkHint: 'lan',
  colorDepth: '32', bulkCompression: 'off',
});

/** Defaults missing legacy fields without losing explicit false preferences. */
export function normalizeHostProfile(profile?: Partial<HostProfile> | null): HostProfile {
  return {
    clipboardDirection: profile?.clipboardDirection ?? DEFAULT_HOST_PROFILE.clipboardDirection,
    colorDepth: profile?.colorDepth ?? DEFAULT_HOST_PROFILE.colorDepth,
    bulkCompression: profile?.bulkCompression ?? DEFAULT_HOST_PROFILE.bulkCompression,
    displayMode: profile?.displayMode ?? DEFAULT_HOST_PROFILE.displayMode,
    resizeMode: profile?.resizeMode ?? DEFAULT_HOST_PROFILE.resizeMode,
    wallpaper: profile?.wallpaper ?? DEFAULT_HOST_PROFILE.wallpaper,
    fullWindowDrag: profile?.fullWindowDrag ?? DEFAULT_HOST_PROFILE.fullWindowDrag,
    menuAnimations: profile?.menuAnimations ?? DEFAULT_HOST_PROFILE.menuAnimations,
    themes: profile?.themes ?? DEFAULT_HOST_PROFILE.themes,
    cursorShadow: profile?.cursorShadow ?? DEFAULT_HOST_PROFILE.cursorShadow,
    cursorBlinking: profile?.cursorBlinking ?? DEFAULT_HOST_PROFILE.cursorBlinking,
    desktopComposition: profile?.desktopComposition ?? DEFAULT_HOST_PROFILE.desktopComposition,
    networkHint: profile?.networkHint ?? DEFAULT_HOST_PROFILE.networkHint,
  };
}

export function normalizeConnection(connection: ConnectionConfig): ConnectionConfig {
  return { ...connection, hostProfile: normalizeHostProfile(connection.hostProfile) };
}
