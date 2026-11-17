import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vitest';
import { DEFAULT_HOST_PROFILE } from '../host-profile';
import { experiencePreset, presetName, VISUAL_OPTIONS, NETWORK_OPTIONS } from '../host-experience';
import { HostProfileFields } from '../../components/connections/HostProfileFields';
import { SessionCanvas } from '../../components/session/SessionCanvas';

it('presets keep display and network choices independent while retaining explicit visual requests', () => {
  const profile = { ...DEFAULT_HOST_PROFILE, displayMode: 'native' as const, resizeMode: 'fixed' as const, networkHint: 'satellite' as const };
  const low = experiencePreset('low');
  const chosen = { ...profile, ...low.hostProfile };
  expect(chosen).toMatchObject({ displayMode: 'native', resizeMode: 'fixed', networkHint: 'satellite', fullWindowDrag: false, cursorBlinking: false });
  expect(low.fontSmoothing).toBe(false);
  expect(presetName(chosen, false)).toBe('low');
  expect(presetName({ ...chosen, themes: true }, false)).toBe('');
  expect(experiencePreset('balanced')).toMatchObject({ fontSmoothing: true, hostProfile: { fullWindowDrag: true, wallpaper: false } });
  expect(Object.values(experiencePreset('quality').hostProfile).every(Boolean)).toBe(true);
});

it('host settings expose all supported visual flags and network hints with linked help', () => {
  const html = renderToStaticMarkup(createElement(HostProfileFields, { profile: { ...DEFAULT_HOST_PROFILE }, fontSmoothing: false, onChange: () => {} }));
  for (const [, label] of [...VISUAL_OPTIONS, ...NETWORK_OPTIONS]) expect(html).toContain(label);
  expect(html).toContain('id="host-fontSmoothing"');
  expect(html).toContain('aria-describedby="experience-help composition-help"');
  expect(html).toContain('Server policy can override');
  expect(html).toContain('not a bandwidth limit');
  expect(html).toContain('data-state="unchecked"');
});

it('connected canvas uses the full viewport without a display-control row', () => {
  const noop = () => {};
  const html = renderToStaticMarkup(createElement(SessionCanvas, {
    status: 'connected', displayMode: 'native', surfaceWidth: 1920, surfaceHeight: 1080,
    subscribeToFrames: () => noop, onKeyDown: noop, onKeyUp: noop, onMouseMove: noop,
    onMouseDown: noop, onMouseUp: noop, onWheel: noop,
  }));
  expect(html).toContain('<canvas');
  expect(html).toContain('image-rendering:pixelated');
  expect(html).not.toContain('<select');
  expect(html).not.toContain('Display size');
});
