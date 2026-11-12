/**
 * Theme resolution seams extracted from `useTheme` (task 3.4
 * characterization): pure moves, rewired into the hook with identical
 * semantics. DOM application (data-theme attribute + class list) stays
 * in the hook.
 */

export type Theme = 'system' | 'light' | 'dark';
export type ResolvedTheme = 'light' | 'dark';

/** Resolve a theme setting to the concrete light/dark variant. */
export function resolveTheme(theme: Theme, prefersDark: boolean): ResolvedTheme {
  if (theme === 'system') {
    return prefersDark ? 'dark' : 'light';
  }
  return theme;
}

/** StatusBar/theme-toggle cycle order: dark → light → system → dark. */
export function nextThemeInCycle(current: Theme): Theme {
  return current === 'dark' ? 'light' : current === 'light' ? 'system' : 'dark';
}
