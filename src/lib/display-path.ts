/**
 * Which path shows the remote picture: the native GPU layer or the web canvas
 * (spec rdp-native-renderer, requirement 1). Asked once; the backend emits
 * `display-path` if a runtime failure switches to the canvas.
 */
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { reportNativeView } from './native-view';

export type DisplayPath = 'native' | 'canvas';

export interface DisplayPathInfo {
  path: DisplayPath;
  reason: string;
}

export const DISPLAY_PATH_EVENT = 'display-path';

const CANVAS: DisplayPathInfo = { path: 'canvas', reason: 'not reported' };

let current: Promise<DisplayPathInfo> | null = null;
let known: DisplayPathInfo = CANVAS;

export function getDisplayPath(): Promise<DisplayPathInfo> {
  current ??= invoke<DisplayPathInfo>('get_display_path')
    .catch(() => CANVAS)
    .then((info) => (known = info));
  return current;
}

/** The last known path, without waiting (canvas until the backend answers). */
export function isNativeDisplay(): boolean {
  return known.path === 'native';
}

export function useDisplayPath(): DisplayPathInfo {
  const [info, setInfo] = useState<DisplayPathInfo>(CANVAS);
  useEffect(() => {
    let live = true;
    void getDisplayPath().then((value) => { if (live) setInfo(value); });
    const unlisten = listen<DisplayPathInfo>(DISPLAY_PATH_EVENT, (event) => {
      current = Promise.resolve(event.payload);
      known = event.payload;
      if (live) setInfo(event.payload);
    }).catch(() => () => {});
    return () => {
      live = false;
      void unlisten.then((stop) => stop());
    };
  }, []);
  return info;
}

/** Keeps `<html class="native-layer">` in step with the path (task 5.1). */
export function syncDisplayPathClass(
  root: HTMLElement = document.documentElement,
  showBackground: () => void = () => reportNativeView(null),
): void {
  const apply = (info: DisplayPathInfo) => {
    root.classList.toggle('native-layer', info.path === 'native');
    // The layer clears to the page background from the start.
    if (info.path === 'native') showBackground();
  };
  void getDisplayPath().then(apply);
  void listen<DisplayPathInfo>(DISPLAY_PATH_EVENT, (event) => {
    current = Promise.resolve(event.payload);
    known = event.payload;
    apply(event.payload);
  }).catch(() => {});
}
