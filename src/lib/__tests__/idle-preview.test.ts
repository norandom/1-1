import { describe, expect, it, vi } from 'vitest';
import { onSessionViewReturn, idleDisplayLabel, displayUpdateAge } from '../idle-preview';
import type { SessionInfo } from '../../types';

describe('idle preview', () => {
  it('dates the paused image using the last paint and tolerates an unknown or future timestamp', () => {
    const paint = '2026-11-18T12:00:00.000Z';
    const time = Date.parse(paint);
    expect(displayUpdateAge(paint, time + 59_999)).toBe('59s ago');
    expect(displayUpdateAge(paint, time + 60_000)).toBe('1m ago');
    expect(displayUpdateAge(paint, time - 500)).toBe('0s ago');
    expect(displayUpdateAge(null, time)).toBeNull();
    expect(displayUpdateAge('invalid', time)).toBeNull();
  });

  it('wakes the visible session once per focus return, including a tab switch', () => {
    const target = new EventTarget() as Window;
    let focused = true;
    const doc = Object.assign(new EventTarget(), { visibilityState: 'visible', hasFocus: () => focused });
    const hostA = vi.fn();
    const hostB = vi.fn();
    const detachA = onSessionViewReturn(target, doc as unknown as Document, hostA);
    expect(hostA).toHaveBeenCalledTimes(1);
    focused = false;
    target.dispatchEvent(new Event('blur'));
    doc.visibilityState = 'hidden';
    doc.dispatchEvent(new Event('visibilitychange'));
    focused = true;
    target.dispatchEvent(new Event('focus'));
    expect(hostA).toHaveBeenCalledTimes(1);
    doc.visibilityState = 'visible';
    doc.dispatchEvent(new Event('visibilitychange'));
    target.dispatchEvent(new Event('focus'));
    expect(hostA).toHaveBeenCalledTimes(2);
    detachA();
    const detachB = onSessionViewReturn(target, doc as unknown as Document, hostB);
    expect(hostB).toHaveBeenCalledTimes(1);
    target.dispatchEvent(new Event('blur'));
    target.dispatchEvent(new Event('focus'));
    expect(hostB).toHaveBeenCalledTimes(2);
    expect(hostA).toHaveBeenCalledTimes(2);
    detachB();
  });

  it('labels only paused connected views and dates them from a decoded update', () => {
    const now = Date.parse('2026-11-17T12:00:00Z');
    const info = { status: 'connected', idlePreviewState: 'suppressed' } as SessionInfo;
    expect(idleDisplayLabel(info, now)).toBe('Idle preview');
    expect(idleDisplayLabel({ ...info, idlePreviewUpdatedAt: '2026-11-17T11:59:31Z' }, now)).toContain('29s ago');
    expect(idleDisplayLabel({ ...info, idlePreviewUpdatedAt: '2026-11-17T11:54:59Z' }, now)).toContain('5m ago');
    expect(idleDisplayLabel({ ...info, idlePreviewState: 'preview' }, now)).toBe('Updating idle preview');
    expect(idleDisplayLabel({ ...info, status: 'disconnected' }, now)).toBeNull();
    expect(idleDisplayLabel({ ...info, idlePreviewState: 'live' }, now)).toBeNull();
  });
});
