import { useCallback, useEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { Loader2, WifiOff } from 'lucide-react';

import { parseFramePacket, tryParseH264Packet } from '../../lib/frame-protocol';
import { H264HardwareDecoder, isH264Keyframe } from '../../lib/h264-decoder';
import {
  compositeFrameRects,
  fitSurfaceToContainer,
  unionDirtyRect,
  type DirtyRect,
} from '../../lib/canvas-compositor';
import type { SessionStatus } from '../../types';

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
  subscribeToFrames: (listener: (frame: Uint8Array) => void) => () => void;
  status: SessionStatus;
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
  subscribeToFrames,
  status,
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
  const h264DecoderRef = useRef<H264HardwareDecoder | null>(null);
  const h264TimestampRef = useRef(0);
  const [displaySize, setDisplaySize] = useState(() => ({
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

  useEffect(() => {
    const draw = () => {
      if (renderScheduledRef.current) {
        return;
      }

      renderScheduledRef.current = true;

      requestAnimationFrame(() => {
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

        if (surface.dirty) {
          draw();
        }
      });
    };

    scheduleDrawRef.current = draw;
  }, [onFramePresented]);

  useEffect(() => {
    const surface = ensureSurface(surfaceWidth, surfaceHeight);
    surface.dirty = unionDirtyRect(surface.dirty, {
      x: 0,
      y: 0,
      width: surface.width,
      height: surface.height,
    });
    scheduleDraw();
  }, [ensureSurface, scheduleDraw, surfaceHeight, surfaceWidth]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    const nextSize = fitSurfaceToContainer(
      container.clientWidth,
      container.clientHeight,
      surfaceWidth,
      surfaceHeight
    );
    setDisplaySize(nextSize);
  }, [surfaceHeight, surfaceWidth]);

  // Initialize H.264 decoder when canvas is available
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const decoder = new H264HardwareDecoder();
    decoder.init(canvas).then((ok) => {
      if (ok) {
        h264DecoderRef.current = decoder;
        console.log('[SessionCanvas] H.264 hardware decoder initialized');
      } else {
        console.log('[SessionCanvas] WebCodecs unavailable, using raw RGBA path');
      }
    });

    return () => {
      decoder.destroy();
      h264DecoderRef.current = null;
    };
  }, []);

  useEffect(() => {
    const unsubscribe = subscribeToFrames((frame) => {
      // Try H.264 path first
      const h264Packet = tryParseH264Packet(frame);
      if (h264Packet && h264DecoderRef.current) {
        const container = containerRef.current;
        if (container) {
          setDisplaySize(
            fitSurfaceToContainer(
              container.clientWidth,
              container.clientHeight,
              h264Packet.surfaceWidth,
              h264Packet.surfaceHeight
            )
          );
        }

        const isKeyframe = isH264Keyframe(h264Packet.h264Data);
        const timestamp = h264TimestampRef.current;
        h264TimestampRef.current += 16667; // ~60fps in microseconds

        h264DecoderRef.current.decode(h264Packet.h264Data, timestamp, isKeyframe);
        onFramePresented?.();
        return;
      }

      // Fall back to raw RGBA frame parsing
      let packet;

      try {
        packet = parseFramePacket(frame);
      } catch (error) {
        console.error('Failed to parse frame packet:', error);
        return;
      }

      const surface = ensureSurface(packet.surfaceWidth, packet.surfaceHeight);
      const container = containerRef.current;
      if (container) {
        setDisplaySize(
          fitSurfaceToContainer(
            container.clientWidth,
            container.clientHeight,
            packet.surfaceWidth,
            packet.surfaceHeight
          )
        );
      }

      compositeFrameRects(surface, packet.rects);

      scheduleDraw();
    });

    return () => {
      unsubscribe();
      renderScheduledRef.current = false;
      surfaceRef.current = null;
    };
  }, [ensureSurface, onFramePresented, scheduleDraw, subscribeToFrames]);

  useEffect(() => {
    if (!containerRef.current) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        const currentSurface = surfaceRef.current;
        setDisplaySize(
          fitSurfaceToContainer(
            width,
            height,
            currentSurface?.width ?? surfaceWidth,
            currentSurface?.height ?? surfaceHeight
          )
        );
        onResize?.(Math.round(width), Math.round(height));
      }
    });

    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, [onResize, surfaceHeight, surfaceWidth]);

  // Auto-focus the canvas when it mounts (so keyboard works immediately)
  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas) {
      canvas.focus();
    }
  }, []);

  // Keyboard events — listen on window so keys work even if canvas loses focus
  useEffect(() => {
    const handleDown = (e: globalThis.KeyboardEvent) => {
      // Only capture when the session view is active (not typing in a dialog/input)
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
      e.preventDefault();
      onKeyDown(e);
    };

    const handleUp = (e: globalThis.KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
      e.preventDefault();
      onKeyUp(e);
    };

    window.addEventListener('keydown', handleDown);
    window.addEventListener('keyup', handleUp);

    return () => {
      window.removeEventListener('keydown', handleDown);
      window.removeEventListener('keyup', handleUp);
    };
  }, [onKeyDown, onKeyUp]);

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

  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      e.preventDefault();
      const bounds = getMouseBounds();
      if (bounds) {
        onWheel(e.nativeEvent, bounds);
      }
    },
    [getMouseBounds, onWheel]
  );

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
            ? 'Failed to connect to the remote desktop'
            : 'The session has been disconnected'}
        </p>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className="relative w-full h-full bg-black overflow-hidden"
      onContextMenu={(e) => e.preventDefault()}
    >
      <canvas
        ref={canvasRef}
        tabIndex={0}
        className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 cursor-default outline-none"
        style={{
          width: `${displaySize.width}px`,
          height: `${displaySize.height}px`,
        }}
        onMouseMove={handleMouseMove}
        onMouseDown={handleMouseDown}
        onMouseUp={handleMouseUp}
        onWheel={handleWheel}
      />
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
  );
}
