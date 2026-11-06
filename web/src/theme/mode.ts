// Light and dark themes: the member's preference (Personal Settings, the profile menu) resolved against the
// browser's when it is `system`, applied as `data-theme` on <html> (tokens.css holds the dark tokens). The
// choice is also kept in localStorage so index.html sets it before the first paint.
import { useSyncExternalStore } from 'react';

export type ThemePreference = 'light' | 'dark' | 'system';
export type Theme = 'light' | 'dark';

/** Read by the boot script in index.html: keep the two in sync. */
export const THEME_STORAGE_KEY = 'oc-theme';

const media = () => window.matchMedia('(prefers-color-scheme: dark)');

let preference: ThemePreference = readStored();
const listeners = new Set<() => void>();

function readStored(): ThemePreference {
  try {
    const v = localStorage.getItem(THEME_STORAGE_KEY);
    return v === 'light' || v === 'dark' ? v : 'system';
  } catch {
    return 'system';
  }
}

export function resolveTheme(p: ThemePreference, systemDark: boolean): Theme {
  return p === 'system' ? (systemDark ? 'dark' : 'light') : p;
}

/** The theme in use now. */
export function currentTheme(): Theme {
  return resolveTheme(preference, media().matches);
}

function render() {
  const theme = currentTheme();
  const root = document.documentElement;
  if (root.dataset.theme !== theme) {
    root.dataset.theme = theme;
    root.style.colorScheme = theme;
  }
  listeners.forEach((l) => l());
}

/** Applies the member's preference (and keeps it for the next page load). */
export function applyThemePreference(p: ThemePreference) {
  preference = p;
  try {
    if (p === 'system') localStorage.removeItem(THEME_STORAGE_KEY);
    else localStorage.setItem(THEME_STORAGE_KEY, p);
  } catch {
    // Private windows may refuse storage: the theme still applies for this page.
  }
  render();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  const m = media();
  m.addEventListener('change', render);
  return () => {
    listeners.delete(listener);
    m.removeEventListener('change', render);
  };
}

/** The theme in use, re-rendering when it changes (a new choice, or the system's when following it). */
export function useTheme(): Theme {
  return useSyncExternalStore(subscribe, currentTheme, () => 'light');
}
