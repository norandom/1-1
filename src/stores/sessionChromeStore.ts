import { create } from 'zustand';
import { createChromeVisibility } from '../lib/session-chrome';

/** Whether the session overlays (top toolbar, overlay bottom bar) are shown. */
export const useSessionChrome = create<{ visible: boolean }>(() => ({ visible: false }));

/** The one controller both overlays drive, so they show and hide together. */
export const sessionChrome = createChromeVisibility((visible) => useSessionChrome.setState({ visible }));
