import { createElement } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { PerformanceHUD } from './PerformanceHUD';
import { useSessionStore } from '../../stores/sessionStore';

afterEach(() => { useSessionStore.getInitialState().showPerformanceHud = false; });

describe('network performance details', () => {
  it('reports unknown without transport measurements or observed codecs', () => {
    useSessionStore.getInitialState().showPerformanceHud = true;
    const html = renderToStaticMarkup(createElement(PerformanceHUD, { fps: 0, latency: 0, resolution: '800x600' }));
    expect(html).toContain('Receive');
    expect(html).toContain('Send');
    expect(html.match(/Unknown/g)).toHaveLength(5); // four values plus explanation
    expect(html).not.toContain('0 B/s');
  });

  it('shows directional byte rates, totals, observation and measurement boundary', () => {
    useSessionStore.getInitialState().showPerformanceHud = true;
    const html = renderToStaticMarkup(createElement(PerformanceHUD, { fps: 20, latency: 30, resolution: '1920x1080',
      network: { rxBytes: 1048576, txBytes: 2048, rxBytesPerSecond: 1536, txBytesPerSecond: 32.4 },
      graphics: { channels: ['egfx'], codecs: ['rfx-progressive', 'clearcodec'] } }));
    expect(html).toContain('1.5 KiB/s');
    expect(html).toContain('32 B/s');
    expect(html).toContain('Received 1.0 MiB');
    expect(html).toContain('Sent 2.0 KiB');
    expect(html).toContain('rfx-progressive, clearcodec');
    expect(html).toContain('Channel: egfx');
    // The display path is asked asynchronously; the first render shows the canvas.
    expect(html).toContain('Renderer: web canvas');
    expect(html).toContain('Excludes TCP/IP headers, retransmissions');
    expect(html).toContain('Totals reset on reconnect');
  });
});
