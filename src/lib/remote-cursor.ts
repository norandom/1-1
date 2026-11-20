/**
 * The remote pointer as the canvas's CSS cursor (see src-tauri/src/rdp/remote_pointer.rs).
 * The system draws it, so moving the mouse repaints nothing.
 */
export interface PointerImage {
  /** PNG data URL in remote desktop pixels. */
  url: string;
  width: number;
  height: number;
  hotspotX: number;
  hotspotY: number;
}

/** Browsers ignore cursor images larger than this. */
export const MAX_CURSOR_SIZE = 128;

export interface CursorGeometry {
  width: number;
  height: number;
  hotspotX: number;
  hotspotY: number;
}

/** Size and hotspot of the pointer at `scale` CSS pixels per remote pixel. */
export function cursorGeometry(image: PointerImage, scale: number): CursorGeometry {
  const fit = Math.min(scale, MAX_CURSOR_SIZE / image.width, MAX_CURSOR_SIZE / image.height);
  const width = Math.max(1, Math.round(image.width * fit));
  const height = Math.max(1, Math.round(image.height * fit));
  return {
    width,
    height,
    hotspotX: Math.min(width - 1, Math.round(image.hotspotX * fit)),
    hotspotY: Math.min(height - 1, Math.round(image.hotspotY * fit)),
  };
}

export function cursorCss(url: string, geometry: Pick<CursorGeometry, 'hotspotX' | 'hotspotY'>): string {
  return `url("${url}") ${geometry.hotspotX} ${geometry.hotspotY}, default`;
}

/** Whether the shape can be used as sent, without rescaling. */
export function needsRescale(image: PointerImage, scale: number): boolean {
  const geometry = cursorGeometry(image, scale);
  return geometry.width !== image.width || geometry.height !== image.height;
}

const scaled = new Map<string, Promise<string>>();
const SCALED_LIMIT = 64;

/** The CSS cursor for `image` shown at `scale`; rescaled shapes are cached. */
export function remoteCursor(image: PointerImage, scale: number): Promise<string> {
  if (!needsRescale(image, scale)) return Promise.resolve(cursorCss(image.url, image));
  const geometry = cursorGeometry(image, scale);
  const key = `${geometry.width}x${geometry.height}|${image.url}`;
  let cursor = scaled.get(key);
  if (!cursor) {
    cursor = rescale(image.url, geometry).then((url) => cursorCss(url, geometry));
    cursor.catch(() => scaled.delete(key));
    if (scaled.size >= SCALED_LIMIT) scaled.delete(scaled.keys().next().value as string);
    scaled.set(key, cursor);
  }
  return cursor;
}

function rescale(url: string, geometry: CursorGeometry): Promise<string> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const canvas = document.createElement('canvas');
      canvas.width = geometry.width;
      canvas.height = geometry.height;
      const ctx = canvas.getContext('2d');
      if (!ctx) {
        reject(new Error('no 2d context'));
        return;
      }
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = 'high';
      ctx.drawImage(img, 0, 0, geometry.width, geometry.height);
      resolve(canvas.toDataURL('image/png'));
    };
    img.onerror = () => reject(new Error('pointer image did not load'));
    img.src = url;
  });
}
