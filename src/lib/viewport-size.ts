export interface ViewportSize { width: number; height: number }

/** CSS viewport to physical RDP pixels; shared by initial and live sizing. */
export function physicalViewport(width: number, height: number, dpr: number): ViewportSize | undefined {
  if (![width, height, dpr].every((value) => Number.isFinite(value) && value > 0)) return undefined;
  const boundedWidth = Math.min(8192, Math.max(200, Math.round(width * dpr)));
  return { width: boundedWidth - boundedWidth % 2, height: Math.min(8192, Math.max(200, Math.round(height * dpr))) };
}
