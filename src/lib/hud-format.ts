/**
 * Performance HUD value formatting extracted from `PerformanceHUD`
 * (task 3.4 characterization): pure classification/formatting moves,
 * rewired into the component with identical semantics.
 */

export function formatBandwidth(bps: number): string {
  if (bps < 1024) return `${bps} B/s`;
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
}

export type HudTrendLevel = 'good' | 'medium' | 'bad';

export function fpsLevel(fps: number): HudTrendLevel {
  return fps >= 24 ? 'good' : fps >= 15 ? 'medium' : 'bad';
}

export function latencyLevel(latency: number): HudTrendLevel {
  return latency < 50 ? 'good' : latency < 150 ? 'medium' : 'bad';
}
