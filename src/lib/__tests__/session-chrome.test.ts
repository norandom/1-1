import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CHROME_IDLE_MS,
  CHROME_LEAVE_MS,
  createChromeVisibility,
  isChromeRevealPoint,
} from '../session-chrome';

describe('session chrome visibility', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const setup = () => {
    const published: boolean[] = [];
    const chrome = createChromeVisibility((visible) => published.push(visible));
    return { chrome, published };
  };

  it('reveals near the top edge only', () => {
    expect(isChromeRevealPoint(0)).toBe(true);
    expect(isChromeRevealPoint(59)).toBe(true);
    expect(isChromeRevealPoint(60)).toBe(false);
  });

  it('shows on reveal and hides after the idle delay, restarting on each reveal', () => {
    const { chrome, published } = setup();
    chrome.reveal();
    vi.advanceTimersByTime(CHROME_IDLE_MS - 1);
    chrome.reveal();
    vi.advanceTimersByTime(CHROME_IDLE_MS - 1);
    expect(published).toEqual([true]);
    vi.advanceTimersByTime(1);
    expect(published).toEqual([true, false]);
  });

  it('stays up while any bar is held and hides shortly after the last release', () => {
    const { chrome, published } = setup();
    chrome.reveal();
    chrome.hold('toolbar-hover');
    chrome.hold('bar-menu');
    vi.advanceTimersByTime(CHROME_IDLE_MS * 3);
    chrome.release('toolbar-hover');
    vi.advanceTimersByTime(CHROME_LEAVE_MS * 3);
    expect(published).toEqual([true]);
    chrome.release('bar-menu');
    vi.advanceTimersByTime(CHROME_LEAVE_MS);
    expect(published).toEqual([true, false]);
  });

  it('ignores duplicate holds and releases', () => {
    const { chrome, published } = setup();
    chrome.hold('bar-focus');
    chrome.hold('bar-focus');
    chrome.release('bar-focus');
    chrome.release('bar-focus');
    vi.advanceTimersByTime(CHROME_LEAVE_MS);
    expect(published).toEqual([true, false]);
  });

  it('a keyboard focus shows both bars without the pointer', () => {
    const { chrome, published } = setup();
    chrome.hold('bar-focus');
    expect(published).toEqual([true]);
    chrome.reset();
    expect(published).toEqual([true, false]);
    vi.advanceTimersByTime(CHROME_IDLE_MS);
    expect(published).toEqual([true, false]);
  });
});
