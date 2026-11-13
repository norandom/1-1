import { Command } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog';
import { Badge } from '@/components/ui/badge';
import { Separator } from '@/components/ui/separator';
import { ScrollArea } from '@/components/ui/scroll-area';
import { SHORTCUT_SECTIONS, resolveShortcutKeys } from '../lib/shortcuts';
import { usePlatformLabel } from '../hooks/usePlatformLabel';

interface Props {
  onClose: () => void;
}

function KeyCap({ children }: { children: string }) {
  const isSymbol = children === '\u2318';
  return (
    <Badge
      variant="outline"
      className="inline-flex items-center justify-center min-w-[24px] h-6 px-1.5 rounded-md text-[11px] font-medium shadow-[0_1px_0_1px_var(--color-surface-0)]"
    >
      {isSymbol ? <Command size={11} /> : children}
    </Badge>
  );
}

export function ShortcutOverlay({ onClose }: Props) {
  // Req 8.5: mod chords display ⌘ on macOS and Ctrl on Windows/Linux.
  const { modLabel } = usePlatformLabel();

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Keyboard Shortcuts</DialogTitle>
          <DialogDescription>
            Navigate faster with keyboard shortcuts
          </DialogDescription>
        </DialogHeader>

        {/* Sections */}
        <ScrollArea className="max-h-[60vh]">
          <div className="space-y-5">
            {SHORTCUT_SECTIONS.map((section, sectionIndex) => (
              <div key={section.title}>
                {sectionIndex > 0 && <Separator className="mb-5" />}
                <h3 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground mb-2.5">
                  {section.title}
                </h3>
                <div className="space-y-2">
                  {section.shortcuts.map((shortcut) => (
                    <div
                      key={shortcut.description}
                      className="flex items-center justify-between py-1"
                    >
                      <span className="text-xs text-muted-foreground">
                        {shortcut.description}
                      </span>
                      <div className="flex items-center gap-1">
                        {resolveShortcutKeys(shortcut.keys, modLabel).map((key, i) => (
                          <KeyCap key={i}>{key}</KeyCap>
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </ScrollArea>
      </DialogContent>
    </Dialog>
  );
}
