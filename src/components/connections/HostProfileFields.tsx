import type { HostProfile } from '../../types';
import { COMPOSITION_HELP, EXPERIENCE_HELP, NETWORK_HELP, NETWORK_OPTIONS, PRESETS, VISUAL_OPTIONS, experiencePreset, presetName, type ExperiencePreset } from '../../lib/host-experience';
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
