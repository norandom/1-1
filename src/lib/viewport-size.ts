import type { HostProfile } from '../types';

/** A remote desktop size in pixels, with the desktop scale factor in percent. */
export interface ViewportSize { width: number; height: number; scale?: number }
export type DisplayScaling = HostProfile['scaling'];

/** CSS viewport to physical RDP pixels; shared by initial and live sizing. */
export function physicalViewport(width: number, height: number, dpr: number): ViewportSize | undefined {
  if (![width, height, dpr].every((value) => Number.isFinite(value) && value > 0)) return undefined;
  const boundedWidth = Math.min(8192, Math.max(200, Math.round(width * dpr)));
  return { width: boundedWidth - boundedWidth % 2, height: Math.min(8192, Math.max(200, Math.round(height * dpr))) };
}

/** Remote pixels per CSS pixel. Low resolution uses one per CSS pixel and lets the display enlarge it. */
export function sessionPixelRatio(scaling: DisplayScaling, dpr: number): number {
  if (!Number.isFinite(dpr) || dpr <= 0) return 1;
  return scaling === 'lowResolution' ? 1 : dpr;
}

/** Desktop scale factor in percent: this display's density for Automatic, otherwise 100%. */
export function desktopScale(scaling: DisplayScaling, dpr: number): number {
  if (scaling !== 'automatic' || !Number.isFinite(dpr) || dpr <= 0) return 100;
  return Math.min(500, Math.max(100, Math.round(dpr * 100)));
}

/** The remote size and scale for a CSS viewport under a host's scaling setting. */
export function remoteViewport(width: number, height: number, scaling: DisplayScaling, dpr: number): ViewportSize | undefined {
  const size = physicalViewport(width, height, sessionPixelRatio(scaling, dpr));
  return size && { ...size, scale: desktopScale(scaling, dpr) };
}
