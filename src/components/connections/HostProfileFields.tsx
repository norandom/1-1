import type { HostProfile } from '../../types';
import { IDLE_PREVIEW_HELP, IDLE_PREVIEW_LABEL } from '../../lib/idle-preview';
import { TRANSFER_PACE_HELP, TRANSFER_PACE_OPTIONS, SCALE_PERCENT_OPTIONS, SCALING_HELP, SCALING_OPTIONS, CLIPBOARD_OPTIONS, CLIPBOARD_HELP, COLOR_DEPTH_HELP, COLOR_DEPTH_OPTIONS, COMPRESSION_HELP, COMPRESSION_OPTIONS, COMPOSITION_HELP, EXPERIENCE_HELP, NETWORK_HELP, NETWORK_OPTIONS, PRESETS, VISUAL_OPTIONS, experiencePreset, presetName, type ExperiencePreset } from '../../lib/host-experience';
import { Switch } from '@/components/ui/switch';
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { describeLocalLayout, layoutChoiceFrom, layoutChoiceValue, type KeyboardLayout, type LocalLayout } from '../../lib/keyboard-status';

interface Props {
  profile: HostProfile;
  fontSmoothing: boolean;
  onChange: (profile: HostProfile, fontSmoothing: boolean) => void;
}
const selectClass = 'rounded border bg-background px-2 py-1 text-sm text-foreground';

export function HostProfileFields({ profile, fontSmoothing, onChange }: Props) {
  return <div className="grid gap-4" data-local-input>
    <fieldset className="grid gap-2">
      <legend className="mb-2 text-sm font-medium">Display</legend>
      <label className="flex items-center justify-between gap-3 text-sm">Display size
        <select className={selectClass} value={profile.displayMode} onChange={(e) => onChange({ ...profile, displayMode: e.target.value as HostProfile['displayMode'] }, fontSmoothing)}>
          <option value="fit">Fit to window</option><option value="native">Native pixels (1:1)</option>
        </select>
      </label>
      <label className="flex items-center justify-between gap-3 text-sm">Remote resolution
        <select className={selectClass} value={profile.resizeMode} onChange={(e) => onChange({ ...profile, resizeMode: e.target.value as HostProfile['resizeMode'] }, fontSmoothing)}>
          <option value="dynamic">Resize with window</option><option value="fixed">Fixed resolution</option>
        </select>
      </label>
      <label className="flex items-center justify-between gap-3 text-sm">Scaling
        <select className={selectClass} aria-describedby="scaling-help" value={profile.scaling} onChange={(e) => onChange({ ...profile, scaling: e.target.value as HostProfile['scaling'] }, fontSmoothing)}>
          {SCALING_OPTIONS.map(([key, label]) => <option key={key} value={key}>{label}</option>)}
        </select>
      </label>
      {profile.scaling === 'custom' && <label className="flex items-center justify-between gap-3 text-sm">Scale
        <select className={selectClass} aria-describedby="scaling-help" value={profile.scalePercent} onChange={(e) => onChange({ ...profile, scalePercent: Number(e.target.value) }, fontSmoothing)}>
          {SCALE_PERCENT_OPTIONS.map((percent) => <option key={percent} value={percent}>{percent}%</option>)}
        </select>
      </label>}
      <p id="scaling-help" className="text-xs text-muted-foreground">{SCALING_HELP}</p>
      <p className="text-xs text-muted-foreground">Display size and automatic resizing apply immediately after saving. Fixed mode uses the saved resolution on the next connection.</p>
    </fieldset>
    <fieldset className="grid gap-2">
      <legend className="mb-2 text-sm font-medium">Graphics</legend>
      <label className="flex items-center justify-between gap-3 text-sm">Color depth
        <select className={selectClass} aria-describedby="color-depth-help" value={profile.colorDepth} onChange={(e) => onChange({ ...profile, colorDepth: e.target.value as HostProfile['colorDepth'] }, fontSmoothing)}>
          {COLOR_DEPTH_OPTIONS.map(([key, label]) => <option key={key} value={key}>{label}</option>)}
        </select>
      </label>
      <p id="color-depth-help" className="text-xs text-muted-foreground">{COLOR_DEPTH_HELP}</p>
      <label className="flex items-center justify-between gap-3 text-sm">Bulk compression
        <select className={selectClass} aria-describedby="compression-help" value={profile.bulkCompression} onChange={(e) => onChange({ ...profile, bulkCompression: e.target.value as HostProfile['bulkCompression'] }, fontSmoothing)}>
          {COMPRESSION_OPTIONS.map(([key, label]) => <option key={key} value={key}>{label}</option>)}
        </select>
      </label>
      <p id="compression-help" className="text-xs text-muted-foreground">{COMPRESSION_HELP}</p>
      <label className="flex items-center justify-between gap-3 text-sm">
        {IDLE_PREVIEW_LABEL}
        <Switch checked={profile.idlePreview} aria-label={IDLE_PREVIEW_LABEL} aria-describedby="idle-preview-help"
          onCheckedChange={(idlePreview) => onChange({ ...profile, idlePreview }, fontSmoothing)} />
      </label>
      <p id="idle-preview-help" className="text-xs text-muted-foreground">{IDLE_PREVIEW_HELP}</p>
    </fieldset>
    <fieldset className="grid gap-2">
      <legend className="mb-2 text-sm font-medium">Clipboard</legend>
      <label className="flex items-center justify-between gap-3 text-sm">Direction
        <select className={selectClass} aria-describedby="clipboard-direction-help" value={profile.clipboardDirection} onChange={(e) => onChange({ ...profile, clipboardDirection: e.target.value as HostProfile['clipboardDirection'] }, fontSmoothing)}>
          {CLIPBOARD_OPTIONS.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select>
      </label>
      <p id="clipboard-direction-help" className="text-xs text-muted-foreground">{CLIPBOARD_HELP}</p>
    </fieldset>
    <fieldset className="grid gap-2">
      <legend className="mb-2 text-sm font-medium">Shared folders</legend>
      <label className="flex items-center justify-between gap-3 text-sm">File transfers
        <select className={selectClass} aria-describedby="transfer-pace-help" value={profile.transferPace} onChange={(e) => onChange({ ...profile, transferPace: e.target.value as HostProfile['transferPace'] }, fontSmoothing)}>
          {TRANSFER_PACE_OPTIONS.map(([key, label]) => <option key={key} value={key}>{label}</option>)}
        </select>
      </label>
      <p id="transfer-pace-help" className="text-xs text-muted-foreground">{TRANSFER_PACE_HELP}</p>
    </fieldset>
    <KeyboardFields profile={profile} onChange={(next) => onChange(next, fontSmoothing)} />
    <fieldset className="grid gap-2">
      <legend className="mb-2 text-sm font-medium">Security</legend>
      <label className="flex items-center justify-between gap-3 text-sm">Security
        <select className={selectClass} aria-describedby="security-help" value={profile.security} onChange={(e) => onChange({ ...profile, security: e.target.value as HostProfile['security'] }, fontSmoothing)}>
          <option value="automatic">Automatic (TLS, NLA)</option>
          <option value="legacy">Legacy (RC4, host not verified)</option>
        </select>
      </label>
      <p id="security-help" className="text-xs text-muted-foreground">{SECURITY_HELP}</p>
    </fieldset>
    <details className="rounded border p-3">
      <summary className="cursor-pointer text-sm font-medium">Visual effects and connection</summary>
      <div className="mt-3 grid gap-3">
        <p id="experience-help" className="text-xs text-muted-foreground">{EXPERIENCE_HELP}</p>
        <label className="flex items-center justify-between gap-3 text-sm">Visual preset
          <select className={selectClass} value={presetName(profile, fontSmoothing)} onChange={(e) => { const preset = experiencePreset(e.target.value as ExperiencePreset); onChange({ ...profile, ...preset.hostProfile }, preset.fontSmoothing); }}>
            <option value="" disabled>Custom</option>{PRESETS.map(([key, label]) => <option key={key} value={key}>{label}</option>)}
          </select>
        </label>
        {VISUAL_OPTIONS.map(([key, label]) => <div key={key} className="flex items-center justify-between gap-3">
          <label htmlFor={`host-${key}`} className="text-sm">{label}</label>
          <Switch id={`host-${key}`} aria-describedby={key === 'desktopComposition' ? 'experience-help composition-help' : 'experience-help'} checked={key === 'fontSmoothing' ? fontSmoothing : profile[key]} onCheckedChange={(checked) => key === 'fontSmoothing' ? onChange(profile, checked) : onChange({ ...profile, [key]: checked }, fontSmoothing)} />
        </div>)}
        <p id="composition-help" className="text-xs text-muted-foreground">{COMPOSITION_HELP}</p>
        <label className="flex items-center justify-between gap-3 text-sm">Connection type
          <select className={selectClass} aria-describedby="network-help" value={profile.networkHint} onChange={(e) => onChange({ ...profile, networkHint: e.target.value as HostProfile['networkHint'] }, fontSmoothing)}>
            {NETWORK_OPTIONS.map(([key, label]) => <option key={key} value={key}>{label}</option>)}
          </select>
        </label>
        <p id="network-help" className="text-xs text-muted-foreground">{NETWORK_HELP}</p>
      </div>
    </details>
  </div>;
}

const SECURITY_HELP = 'Automatic encrypts with TLS and signs in with NLA where the host offers it. Legacy is for hosts that refuse both: RC4 encryption that is easy to break and no check that the host is the right one, so someone on the network path could read the session and the password. Use it only on a network you trust; shared folders stay off.';

const KEYS_HELP = 'Automatic sends typed characters as characters, so they match your layout whatever the remote uses; shortcuts and named keys go as keys. Physical keys sends every key by its position, as older versions did; use it for servers that ignore character input.';

/** Spec rdp-keyboard-layout, task 1.3. */
function KeyboardFields({ profile, onChange }: { profile: HostProfile; onChange: (profile: HostProfile) => void }) {
  const [layouts, setLayouts] = useState<KeyboardLayout[]>([]);
  const [local, setLocal] = useState<LocalLayout | null | undefined>(undefined);
  useEffect(() => {
    let live = true;
    invoke<KeyboardLayout[]>('keyboard_layouts').then((list) => { if (live) setLayouts(list); }).catch(() => {});
    invoke<LocalLayout | null>('detect_keyboard_layout').then((found) => { if (live) setLocal(found); }).catch(() => { if (live) setLocal(null); });
    return () => { live = false; };
  }, []);
  return <fieldset className="grid gap-2">
    <legend className="mb-2 text-sm font-medium">Keyboard</legend>
    <label className="flex items-center justify-between gap-3 text-sm">Layout
      <select className={selectClass} aria-describedby="keyboard-layout-help" value={layoutChoiceValue(profile.keyboardLayout)} onChange={(e) => onChange({ ...profile, keyboardLayout: layoutChoiceFrom(e.target.value) })}>
        <option value="automatic">Automatic (this computer's layout)</option>
        <option value="serverDefault">Server default</option>
        {layouts.map((layout) => <option key={layout.klid} value={layoutChoiceValue(layout.klid)}>{layout.name}</option>)}
      </select>
    </label>
    <p id="keyboard-layout-help" className="text-xs text-muted-foreground">
      {local === undefined ? 'Detecting this computer\'s layout…' : `Detected: ${describeLocalLayout(local)}`}
    </p>
    <label className="flex items-center justify-between gap-3 text-sm">Keys
      <select className={selectClass} aria-describedby="keyboard-keys-help" value={profile.keyboardKeys} onChange={(e) => onChange({ ...profile, keyboardKeys: e.target.value as HostProfile['keyboardKeys'] })}>
        <option value="automatic">Automatic</option>
        <option value="physical">Physical keys</option>
      </select>
    </label>
    <p id="keyboard-keys-help" className="text-xs text-muted-foreground">{KEYS_HELP}</p>
    <div className="flex items-center justify-between gap-3">
      <label htmlFor="host-superAsCtrl" className="text-sm">Super as Ctrl (Super+C copies; Super alone opens Start)</label>
      <Switch id="host-superAsCtrl" checked={profile.superAsCtrl} onCheckedChange={(superAsCtrl) => onChange({ ...profile, superAsCtrl })} />
    </div>
  </fieldset>;
}

