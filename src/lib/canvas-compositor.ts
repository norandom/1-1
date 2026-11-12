import type { FrameRect } from './frame-protocol';

/**
 * Canvas compositing seams extracted from `SessionCanvas` (task 3.4
 * characterization): pure functions for letterboxed fit-to-container
 * scaling and dirty-rectangle compositing onto an RGBA surface buffer.
 * The component call sites were rewired to these moves with identical
 * semantics.
 */

export interface DirtyRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface CompositingSurface {
  width: number;
  height: number;
  pixels: Uint8ClampedArray;
  dirty: DirtyRect | null;
}

export function fitSurfaceToContainer(
  containerWidth: number,
  containerHeight: number,
  surfaceWidth: number,
  surfaceHeight: number
) {
  if (containerWidth <= 0 || containerHeight <= 0 || surfaceWidth <= 0 || surfaceHeight <= 0) {
    return {
      width: Math.max(surfaceWidth, 1),
      height: Math.max(surfaceHeight, 1),
    };
  }

  const scale = Math.min(
    containerWidth / surfaceWidth,
    containerHeight / surfaceHeight
  );

  return {
    width: Math.max(1, Math.floor(surfaceWidth * scale)),
    height: Math.max(1, Math.floor(surfaceHeight * scale)),
  };
}

export function unionDirtyRect(current: DirtyRect | null, next: DirtyRect): DirtyRect {
  if (!current) {
    return next;
  }

  const left = Math.min(current.x, next.x);
  const top = Math.min(current.y, next.y);
  const right = Math.max(current.x + current.width, next.x + next.width);
  const bottom = Math.max(current.y + current.height, next.y + next.height);

  return {
    x: left,
    y: top,
    width: right - left,
    height: bottom - top,
  };
}

/**
 * Composite parsed frame rects onto `surface` in order: each rect's pixels
 * are copied row by row into the surface buffer (fast path replaces the
 * whole buffer when the rect covers the exact surface with a full-length
 * payload), the copy region is clamped to the surface bounds, and every
 * copied rect is unioned into the surface's pending dirty region.
 */
export function compositeFrameRects(
  surface: CompositingSurface,
  rects: FrameRect[]
): void {
  for (const rect of rects) {
    if (rect.width === 0 || rect.height === 0) {
      continue;
    }

    const rowBytes = rect.width * 4;
    const rectBottom = Math.min(rect.y + rect.height, surface.height);
    const rectRight = Math.min(rect.x + rect.width, surface.width);
    const copyWidth = rectRight - rect.x;
    const copyHeight = rectBottom - rect.y;

    if (copyWidth <= 0 || copyHeight <= 0) {
      continue;
    }

    if (
      rect.x === 0 &&
      rect.y === 0 &&
      rect.width === surface.width &&
      rect.height === surface.height &&
      rect.pixels.length === surface.pixels.length
    ) {
      surface.pixels.set(rect.pixels);
    } else {
      for (let row = 0; row < copyHeight; row += 1) {
        const srcStart = row * rowBytes;
        const srcEnd = srcStart + copyWidth * 4;
        const dstStart =
          ((rect.y + row) * surface.width + rect.x) * 4;

        surface.pixels.set(rect.pixels.subarray(srcStart, srcEnd), dstStart);
      }
    }

    surface.dirty = unionDirtyRect(surface.dirty, {
      x: rect.x,
      y: rect.y,
      width: copyWidth,
      height: copyHeight,
    });
  }
}
