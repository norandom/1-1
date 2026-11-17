import type { HostProfile } from '../../types';
import { IDLE_PREVIEW_HELP, IDLE_PREVIEW_LABEL } from '../../lib/idle-preview';
import { CLIPBOARD_OPTIONS, CLIPBOARD_HELP, COLOR_DEPTH_HELP, COLOR_DEPTH_OPTIONS, COMPRESSION_HELP, COMPRESSION_OPTIONS, COMPOSITION_HELP, EXPERIENCE_HELP, NETWORK_HELP, NETWORK_OPTIONS, PRESETS, VISUAL_OPTIONS, experiencePreset, presetName, type ExperiencePreset } from '../../lib/host-experience';
import { Switch } from '@/components/ui/switch';

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
