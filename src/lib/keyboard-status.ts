/**
 * The keyboard layout a session reported at connect, for the status bar and
 * the HUD (spec rdp-keyboard-layout, requirement 5).
 */
export type LayoutSource = 'detected' | 'closestMatch' | 'override' | 'serverDefault' | 'notDetected';

export interface ReportedLayout {
  /** The Windows layout ID sent to the server; 0 for none. */
  klid: number;
  short: string | null;
  name: string;
  source: LayoutSource;
  /** The local layout it came from, by its own name. */
  local: string | null;
}

/** The status bar's short label: "DE", or "—" when nothing was reported. */
export function keyboardLabel(layout: ReportedLayout): string {
  return layout.short ?? '—';
}

/** The tooltip: the layout and why it was chosen. */
export function keyboardDescription(layout: ReportedLayout): string {
  switch (layout.source) {
    case 'detected':
      return `${layout.name}, the local layout`;
    case 'closestMatch':
      return `${layout.name}, closest Windows layout for ${layout.local ?? 'the local layout'}; typed characters still match the local layout`;
    case 'override':
      return `${layout.name}, set for this host`;
    case 'serverDefault':
      return "No layout reported: the remote uses its own (set for this host)";
    case 'notDetected':
      return `No layout reported: ${layout.local ? `no Windows layout for ${layout.local}` : 'the local layout could not be detected'}; the remote uses its own`;
  }
}
