import { useRef, useState } from 'react';
import { Settings2 } from 'lucide-react';
import type { ConnectionConfig, HostProfile } from '../../types';
import { useConnectionStore } from '../../stores/connectionStore';
import { selectIsWriteBlocked, useLibraryStore } from '../../stores/libraryStore';
import { normalizeHostProfile } from '../../lib/host-profile';
import { CLIPBOARD_OPTIONS, CLIPBOARD_HELP, COLOR_DEPTH_HELP, COLOR_DEPTH_OPTIONS, COMPRESSION_HELP, COMPRESSION_OPTIONS, COMPOSITION_HELP, EXPERIENCE_HELP, NETWORK_HELP, NETWORK_OPTIONS, PRESETS, VISUAL_OPTIONS, experiencePreset } from '../../lib/host-experience';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuSub, DropdownMenuSubTrigger, DropdownMenuSubContent, DropdownMenuPortal, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuCheckboxItem, DropdownMenuItem } from '@/components/ui/dropdown-menu';

export function SessionOptionsMenu({ connection }: { connection: ConnectionConfig }) {
  const update = useConnectionStore((s) => s.updateConnection);
  const blocked = useLibraryStore(selectIsWriteBlocked);
  const busy = useRef(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const profile = normalizeHostProfile(connection.hostProfile);
  const save = async (patch: { hostProfile?: Partial<HostProfile>; fontSmoothing?: boolean }) => {
    if (busy.current || blocked) return;
    busy.current = true;
    setSaving(true);
    setError(null);
    try { await update(connection.id, patch); }
    catch (e) { setError(`Settings not saved: ${e instanceof Error ? e.message : String(e)}`); }
    finally { busy.current = false; setSaving(false); }
  };
  const disabled = saving || blocked;
  return <div className="relative ml-auto shrink-0 px-1" data-local-input>
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button id="session-options-trigger" variant="ghost" size="icon-sm" aria-label={`Session options for ${connection.name}`} aria-keyshortcuts="Control+Alt+Shift+P" title="Session options (Ctrl+Alt+Shift+P)" className="h-7 w-8">
          <Settings2 size={15} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64" data-local-input onEscapeKeyDown={(event) => event.stopPropagation()}>
        <DropdownMenuLabel>Saved settings for {connection.name}</DropdownMenuLabel>
        {blocked && <p role="alert" className="px-2 py-1 text-xs text-destructive">Settings are read-only during storage recovery.</p>}
        {saving && <p role="status" className="px-2 py-1 text-xs">Saving…</p>}
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>Clipboard</DropdownMenuSubTrigger>
          <DropdownMenuPortal>
            <DropdownMenuSubContent className="w-72" data-local-input onEscapeKeyDown={(event) => event.stopPropagation()}>
              <DropdownMenuRadioGroup value={profile.clipboardDirection} onValueChange={(clipboardDirection) => void save({ hostProfile: { clipboardDirection: clipboardDirection as HostProfile['clipboardDirection'] } })}>
                {CLIPBOARD_OPTIONS.map(([value, label]) => <DropdownMenuRadioItem key={value} value={value} disabled={disabled} onSelect={(event) => event.preventDefault()}>{label}</DropdownMenuRadioItem>)}
              </DropdownMenuRadioGroup>
              <p className="px-2 py-1 text-xs text-muted-foreground">{CLIPBOARD_HELP}</p>
            </DropdownMenuSubContent>
          </DropdownMenuPortal>
        </DropdownMenuSub>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>Display</DropdownMenuSubTrigger>
          <DropdownMenuPortal>
          <DropdownMenuSubContent className="w-72" data-local-input onEscapeKeyDown={(event) => event.stopPropagation()}>
            <DropdownMenuLabel>Display size</DropdownMenuLabel>
            <DropdownMenuRadioGroup value={profile.displayMode} onValueChange={(displayMode) => void save({ hostProfile: { displayMode: displayMode as HostProfile['displayMode'] } })}>
              <DropdownMenuRadioItem disabled={disabled} value="fit" onSelect={(e) => e.preventDefault()}>Fit to window</DropdownMenuRadioItem>
              <DropdownMenuRadioItem disabled={disabled} value="native" onSelect={(e) => e.preventDefault()}>Native pixels (1:1)</DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Remote resolution</DropdownMenuLabel>
            <DropdownMenuRadioGroup value={profile.resizeMode} onValueChange={(resizeMode) => void save({ hostProfile: { resizeMode: resizeMode as HostProfile['resizeMode'] } })}>
              <DropdownMenuRadioItem disabled={disabled} value="dynamic" onSelect={(e) => e.preventDefault()}>Resize with window</DropdownMenuRadioItem>
              <DropdownMenuRadioItem disabled={disabled} value="fixed" onSelect={(e) => e.preventDefault()}>Fixed resolution</DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
            <p className="px-2 py-2 text-xs text-muted-foreground">Applies immediately. Fixed mode stops automatic resizing; reconnect to use the saved {connection.displayWidth ?? 1920} × {connection.displayHeight ?? 1080} resolution.</p>
          </DropdownMenuSubContent>
          </DropdownMenuPortal>
        </DropdownMenuSub>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>Graphics</DropdownMenuSubTrigger>
          <DropdownMenuPortal>
          <DropdownMenuSubContent className="w-80 max-h-(--radix-dropdown-menu-content-available-height) overflow-y-auto" data-local-input onEscapeKeyDown={(event) => event.stopPropagation()}>
            <DropdownMenuLabel>Color depth</DropdownMenuLabel>
            <DropdownMenuRadioGroup value={profile.colorDepth} onValueChange={(colorDepth) => void save({ hostProfile: { colorDepth: colorDepth as HostProfile['colorDepth'] } })}>
              {COLOR_DEPTH_OPTIONS.map(([key, label]) => <DropdownMenuRadioItem key={key} value={key} disabled={disabled} onSelect={(e) => e.preventDefault()}>{label}</DropdownMenuRadioItem>)}
            </DropdownMenuRadioGroup>
            <p className="px-2 py-2 text-xs text-muted-foreground">{COLOR_DEPTH_HELP}</p>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Bulk compression</DropdownMenuLabel>
            <DropdownMenuRadioGroup value={profile.bulkCompression} onValueChange={(bulkCompression) => void save({ hostProfile: { bulkCompression: bulkCompression as HostProfile['bulkCompression'] } })}>
              {COMPRESSION_OPTIONS.map(([key, label]) => <DropdownMenuRadioItem key={key} value={key} disabled={disabled} onSelect={(e) => e.preventDefault()}>{label}</DropdownMenuRadioItem>)}
            </DropdownMenuRadioGroup>
            <p className="px-2 py-2 text-xs text-muted-foreground">{COMPRESSION_HELP}</p>
          </DropdownMenuSubContent>
          </DropdownMenuPortal>
        </DropdownMenuSub>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>Visual effects</DropdownMenuSubTrigger>
          <DropdownMenuPortal>
          <DropdownMenuSubContent className="w-80" data-local-input onEscapeKeyDown={(event) => event.stopPropagation()}>
            <DropdownMenuLabel>Visual preset</DropdownMenuLabel>
            {PRESETS.map(([key, label]) => <DropdownMenuItem key={key} disabled={disabled} onSelect={(e) => { e.preventDefault(); void save(experiencePreset(key)); }}>{label}</DropdownMenuItem>)}
            <DropdownMenuSeparator />
            {VISUAL_OPTIONS.map(([key, label]) => <DropdownMenuCheckboxItem key={key} disabled={disabled} checked={key === 'fontSmoothing' ? connection.fontSmoothing : profile[key]} onSelect={(e) => e.preventDefault()} onCheckedChange={(value) => void save(key === 'fontSmoothing' ? { fontSmoothing: value } : { hostProfile: { [key]: value } })}>{label}</DropdownMenuCheckboxItem>)}
            <p className="px-2 py-2 text-xs text-muted-foreground">{EXPERIENCE_HELP} {COMPOSITION_HELP}</p>
          </DropdownMenuSubContent>
          </DropdownMenuPortal>
        </DropdownMenuSub>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>Connection</DropdownMenuSubTrigger>
          <DropdownMenuPortal>
          <DropdownMenuSubContent className="w-72" data-local-input onEscapeKeyDown={(event) => event.stopPropagation()}>
            <DropdownMenuLabel>Connection type</DropdownMenuLabel>
            <DropdownMenuRadioGroup value={profile.networkHint} onValueChange={(networkHint) => void save({ hostProfile: { networkHint: networkHint as HostProfile['networkHint'] } })}>
              {NETWORK_OPTIONS.map(([key, label]) => <DropdownMenuRadioItem key={key} value={key} disabled={disabled} onSelect={(e) => e.preventDefault()}>{label}</DropdownMenuRadioItem>)}
            </DropdownMenuRadioGroup>
            <p className="px-2 py-2 text-xs text-muted-foreground">{NETWORK_HELP} Applies on the next connection.</p>
          </DropdownMenuSubContent>
          </DropdownMenuPortal>
        </DropdownMenuSub>
      </DropdownMenuContent>
    </DropdownMenu>
    {error && <div role="alert" className="absolute right-0 top-full z-[60] mt-1 w-72 rounded border border-destructive bg-background p-3 text-xs text-destructive shadow-lg">
      {error}<button type="button" aria-label="Dismiss settings error" className="ml-2 underline" onClick={() => setError(null)}>Dismiss</button>
    </div>}
  </div>;
}
