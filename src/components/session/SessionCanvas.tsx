import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { Loader2, WifiOff } from 'lucide-react';

import { parseFramePacket } from '../../lib/frame-protocol';
import { physicalViewport } from '../../lib/viewport-size';
import { subscribeCanvasKeyboard } from './canvas-keyboard';
import type { FrameListener } from '../../lib/session-runtime';
import {
  compositeFrameRects,
  layoutSurface,
  type DisplayMode,
  unionDirtyRect,
  type DirtyRect,
} from '../../lib/canvas-compositor';
import type { SessionStatus } from '../../types';
import { subscribeCanvasWheel } from './canvas-wheel';

interface MouseSurfaceBounds {
  rect: DOMRect;
  surfaceWidth: number;
  surfaceHeight: number;
}

interface SurfaceBuffer {
  width: number;
  height: number;
  pixels: Uint8ClampedArray;
  imageData: ImageData | null;
  dirty: DirtyRect | null;
}

interface Props {
  displayMode?: DisplayMode;
  inputBlocked?: boolean;
  overlays?: React.ReactNode;
  subscribeToFrames: (listener: FrameListener) => () => void;
  displayStalled?: boolean;
  onRefreshDisplay?: () => void;
  status: SessionStatus;
  lastError?: string | null;
  reconnectAttempts?: number;
  maxReconnectAttempts?: number;
  surfaceWidth: number;
  surfaceHeight: number;
  onFramePresented?: () => void;
  onKeyDown: (e: globalThis.KeyboardEvent) => void;
  onKeyUp: (e: globalThis.KeyboardEvent) => void;
  onMouseMove: (e: globalThis.MouseEvent, bounds: MouseSurfaceBounds) => void;
  onMouseDown: (e: globalThis.MouseEvent, bounds: MouseSurfaceBounds) => void;
  onMouseUp: (e: globalThis.MouseEvent, bounds: MouseSurfaceBounds) => void;
  onWheel: (e: globalThis.WheelEvent, bounds: MouseSurfaceBounds) => void;
  onResize?: (width: number, height: number) => void;
}

export function SessionCanvas({
  displayMode = 'fit',
  inputBlocked = false,
  overlays,
  subscribeToFrames,
  displayStalled = false,
  onRefreshDisplay,
  status,
  lastError,
  reconnectAttempts = 0,
  maxReconnectAttempts = 5,
  surfaceWidth,
  surfaceHeight,
  onFramePresented,
  onKeyDown,
  onKeyUp,
  onMouseMove,
  onMouseDown,
  onMouseUp,
  onWheel,
  onResize,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<SurfaceBuffer | null>(null);
  const renderScheduledRef = useRef(false);
  const pendingConsumptionRef = useRef<(() => void) | undefined>(undefined);
  const animationFrameRef = useRef<number | null>(null);
  const displayModeRef = useRef<DisplayMode>('fit');
  const [displaySize, setDisplaySize] = useState(() => ({
    left: 0,
    top: 0,
    width: surfaceWidth,
    height: surfaceHeight,
  }));

  const ensureSurface = useCallback((width: number, height: number) => {
    const nextWidth = Math.max(1, Math.round(width));
    const nextHeight = Math.max(1, Math.round(height));
    const current = surfaceRef.current;

    if (current && current.width === nextWidth && current.height === nextHeight) {
      return current;
    }

    const pixels = new Uint8ClampedArray(nextWidth * nextHeight * 4);
    const imageData = new ImageData(pixels, nextWidth, nextHeight);
    const nextSurface: SurfaceBuffer = {
      width: nextWidth,
      height: nextHeight,
      pixels,
      imageData,
      dirty: {
        x: 0,
        y: 0,
        width: nextWidth,
        height: nextHeight,
      },
    };

    surfaceRef.current = nextSurface;

    const canvas = canvasRef.current;
    if (canvas) {
      canvas.width = nextWidth;
      canvas.height = nextHeight;
    }

    return nextSurface;
  }, []);

  // Latest-ref indirection for the recursive draw scheduler: the rAF callback
  // must be able to re-schedule itself when new dirty rects arrive mid-draw,
  // but a useCallback cannot reference its own binding before declaration.
  // The effect below owns the implementation; scheduleDraw stays referentially
  // stable so frame-subscription effects do not resubscribe on every render.
  const scheduleDrawRef = useRef<() => void>(() => undefined);

  const scheduleDraw = useCallback(() => {
    scheduleDrawRef.current();
  }, []);

  useLayoutEffect(() => {
    const draw = () => {
      if (renderScheduledRef.current) {
        return;
      }

      renderScheduledRef.current = true;

      animationFrameRef.current = requestAnimationFrame(() => {
        animationFrameRef.current = null;
        renderScheduledRef.current = false;

        const canvas = canvasRef.current;
        const surface = surfaceRef.current;
        if (!canvas || !surface?.imageData || !surface.dirty) {
          return;
        }

        if (canvas.width !== surface.width || canvas.height !== surface.height) {
          canvas.width = surface.width;
          canvas.height = surface.height;
        }

        const ctx = canvas.getContext('2d');
        if (!ctx) {
          return;
        }

        ctx.imageSmoothingEnabled = false;

        const dirty = surface.dirty;
        surface.dirty = null;

        ctx.putImageData(
          surface.imageData,
          0,
          0,
          dirty.x,
          dirty.y,
          dirty.width,
          dirty.height
        );

        onFramePresented?.();
        const consumed = pendingConsumptionRef.current;
        pendingConsumptionRef.current = undefined;
        consumed?.();

        if (surface.dirty) {
          draw();
        }
      });
    };

    scheduleDrawRef.current = draw;
  }, [onFramePresented]);

  useLayoutEffect(() => {
    const surface = ensureSurface(surfaceWidth, surfaceHeight);
    surface.dirty = unionDirtyRect(surface.dirty, {
      x: 0,
      y: 0,
      width: surface.width,
      height: surface.height,
    });
    scheduleDraw();
  }, [ensureSurface, scheduleDraw, surfaceHeight, surfaceWidth, status]);

  const updateLayout = useCallback(() => {
    const container = containerRef.current;
    const surface = surfaceRef.current;
    if (!container || !surface) return;
    const rect = container.getBoundingClientRect();
    setDisplaySize(layoutSurface({
      width: container.clientWidth, height: container.clientHeight,
      left: rect.left, top: rect.top,
      scrollLeft: container.scrollLeft, scrollTop: container.scrollTop,
    }, surface.width, surface.height, displayModeRef.current, window.devicePixelRatio));
  }, []);

  useLayoutEffect(() => {
    displayModeRef.current = displayMode;
    if (containerRef.current) {
      containerRef.current.scrollLeft = 0;
      containerRef.current.scrollTop = 0;
    }
    updateLayout();
  }, [displayMode, surfaceWidth, surfaceHeight, updateLayout, status]);

  useEffect(() => {
    const unsubscribe = subscribeToFrames((frame, consumed) => {
      // Decoded RGBA bitmap packets only (kinds 1/2, Req 4.4/9.4): the
      // H.264 kind-3 path was removed with its encoder/decoder (task 4.4).
      let packet;

      try {
        packet = parseFramePacket(frame);
      } catch (error) {
        console.error('Failed to parse frame packet:', error);
        return;
      }

      const surface = ensureSurface(packet.surfaceWidth, packet.surfaceHeight);
      updateLayout();

      compositeFrameRects(surface, packet.rects);
      pendingConsumptionRef.current = consumed;

      scheduleDraw();
    });

    return () => {
      unsubscribe();
      if (animationFrameRef.current !== null) cancelAnimationFrame(animationFrameRef.current);
      animationFrameRef.current = null;
      pendingConsumptionRef.current = undefined;
      renderScheduledRef.current = false;
      surfaceRef.current = null;
    };
  }, [ensureSurface, onFramePresented, scheduleDraw, subscribeToFrames, updateLayout]);

  useEffect(() => {
    if (!containerRef.current) return;

    let previousWidth = -1;
    let previousHeight = -1;
    const resized = () => {
      const container = containerRef.current;
      if (!container) return;
      updateLayout();
      // Include local scrollbars so switching modes cannot resize the server
      // and feed back into the remote surface dimensions.
      const size = physicalViewport(container.offsetWidth, container.offsetHeight, window.devicePixelRatio);
      if (!size) return;
      const { width, height } = size;
      if (width !== previousWidth || height !== previousHeight) {
        previousWidth = width;
        previousHeight = height;
        onResize?.(width, height);
      }
    };
    let query: MediaQueryList;
    const densityChanged = () => {
      query?.removeEventListener('change', densityChanged);
      query = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      query.addEventListener('change', densityChanged);
      resized();
    };
    const observer = new ResizeObserver(resized);
    observer.observe(containerRef.current);
    densityChanged();
    window.addEventListener('resize', densityChanged);
    return () => {
      observer.disconnect();
      query.removeEventListener('change', densityChanged);
      window.removeEventListener('resize', densityChanged);
    };
  }, [onResize, updateLayout, status]);

  // Auto-focus the canvas when it mounts (so keyboard works immediately)
  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas) {
      canvas.focus();
    }
  }, [status]);

  // Keyboard events — listen on window so keys work even if canvas loses focus
  useEffect(() => {
    if (inputBlocked) return;
    return subscribeCanvasKeyboard(window, document.getElementById('session-options-trigger'), onKeyDown, onKeyUp);
  }, [onKeyDown, onKeyUp, status, inputBlocked]);

  const getMouseBounds = useCallback((): MouseSurfaceBounds | null => {
    const canvas = canvasRef.current;
    const surface = surfaceRef.current;
    if (!canvas || !surface) {
      return null;
    }

    return {
      rect: canvas.getBoundingClientRect(),
      surfaceWidth: surface.width,
      surfaceHeight: surface.height,
    };
  }, []);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      const bounds = getMouseBounds();
      if (bounds) {
        onMouseMove(e.nativeEvent, bounds);
      }
    },
    [getMouseBounds, onMouseMove]
  );

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      const bounds = getMouseBounds();
      if (bounds) {
        onMouseDown(e.nativeEvent, bounds);
      }
      canvasRef.current?.focus();
    },
    [getMouseBounds, onMouseDown]
  );

  const handleMouseUp = useCallback(
    (e: React.MouseEvent) => {
      const bounds = getMouseBounds();
      if (bounds) {
        onMouseUp(e.nativeEvent, bounds);
      }
    },
    [getMouseBounds, onMouseUp]
  );

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    // React wheel handlers are passive; use a native non-passive listener so
    // forwarding a remote wheel cannot also pan the local native-pixel view.
    return subscribeCanvasWheel(canvas, (event) => {
      const bounds = getMouseBounds();
      if (bounds) {
        onWheel(event, bounds);
      }
    });
  }, [getMouseBounds, onWheel, status]);

  if (status === 'connecting') {
    return (
      <div className="flex flex-col items-center justify-center h-full bg-[var(--color-surface-0)]">
        <motion.div
          animate={{ rotate: 360 }}
          transition={{ duration: 1, repeat: Infinity, ease: 'linear' }}
        >
          <Loader2 size={36} className="text-[var(--color-accent)]" />
        </motion.div>
        <p className="text-sm text-[var(--color-text-secondary)] mt-4">
          Establishing connection...
        </p>
      </div>
    );
  }

  if (status === 'error' || status === 'disconnected') {
    return (
      <div className="flex flex-col items-center justify-center h-full bg-[var(--color-surface-0)]">
        <div className="flex items-center justify-center w-16 h-16 rounded-2xl bg-[var(--color-surface-2)] mb-4">
          <WifiOff size={28} className="text-[var(--color-danger)]" />
        </div>
        <p className="text-sm font-medium text-[var(--color-text-primary)] mb-1">
          {status === 'error' ? 'Connection Error' : 'Disconnected'}
        </p>
        <p className="text-xs text-[var(--color-text-muted)]">
          {status === 'error'
            ? (lastError ?? 'Failed to connect to the remote desktop')
            : 'The session has been disconnected'}
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col w-full h-full bg-black">
      <div className="relative flex flex-1 min-h-0 flex-col overflow-hidden">
        <div
          ref={containerRef}
          className="relative flex-1 min-h-0 w-full"
          style={{ overflow: displayMode === 'native' ? 'auto' : 'hidden' }}
          onScroll={updateLayout}
          onContextMenu={(e) => e.preventDefault()}
        >
          <canvas
            ref={canvasRef}
            tabIndex={0}
            className="absolute cursor-default outline-none"
            style={{
              left: displaySize.left,
              top: displaySize.top,
              imageRendering: displayMode === 'native' ? 'pixelated' : 'auto',
              width: `${displaySize.width}px`,
              height: `${displaySize.height}px`,
            }}
            onMouseMove={handleMouseMove}
            onMouseDown={handleMouseDown}
            onMouseUp={handleMouseUp}
          />
          {displayStalled && (
            <div role="alert" className="absolute top-3 left-3 right-3 flex items-center justify-between gap-3 rounded bg-amber-950 px-4 py-3 text-white">
              <span>Display updates paused. Refresh to resume.</span>
              <button type="button" className="rounded border px-3 py-1" onKeyDown={(event) => event.stopPropagation()} onKeyUp={(event) => event.stopPropagation()} onClick={onRefreshDisplay}>Refresh display</button>
            </div>
          )}
          {status === 'reconnecting' && (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              className="absolute inset-0 flex items-center justify-center bg-black/70 backdrop-blur-sm"
            >
              <div className="text-center">
                <motion.div
                  animate={{ rotate: 360 }}
                  transition={{ duration: 1, repeat: Infinity, ease: 'linear' }}
                  className="inline-block"
                >
                  <Loader2 size={28} className="text-[var(--color-warning)]" />
                </motion.div>
                <p className="text-sm text-white mt-3">
                  Reconnecting...{reconnectAttempts > 0 && ` (attempt ${reconnectAttempts}/${maxReconnectAttempts})`}
                </p>
                {reconnectAttempts > 0 && (
                  <p className="text-xs text-white/60 mt-1">
                    Retrying in {Math.min(1 << reconnectAttempts, 30)}s...
                  </p>
                )}
              </div>
            </motion.div>
          )}
        </div>
        {/* Keep floating chrome clipped to the viewport, outside native-pixel scrolling. */}
        <div className="pointer-events-none absolute inset-0 overflow-hidden [&>*]:pointer-events-auto">
          {overlays}
        </div>
      </div>
    </div>
  );
}
