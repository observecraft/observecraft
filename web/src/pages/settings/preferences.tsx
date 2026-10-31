// Personal Settings preferences: read once per session, applied to the whole app (accessibility
// attributes on <html>, clock format) and to `/` (the home page).
import { useEffect } from 'react';
import { Navigate } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { DEFAULT_PREFERENCES, getPreferences, setPreferences, type Preferences } from '../../api/identity/personal';
import { setClockFormat } from '../../lib/format';
import './preferences.css';

const KEY = ['identity', 'preferences'] as const;

export const usePreferences = () => useQuery({ queryKey: KEY, queryFn: getPreferences, staleTime: 5 * 60_000, retry: 1 });

export function useSavePreferences() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: setPreferences, onSuccess: (p) => qc.setQueryData(KEY, p) });
}

function apply(p: Preferences) {
  const root = document.documentElement;
  const flag = (name: string, on: boolean) => (on ? root.setAttribute(name, '') : root.removeAttribute(name));
  flag('data-reduced-motion', p.reducedMotion);
  flag('data-high-contrast', p.highContrast);
  flag('data-underline-links', p.underlineLinks);
  flag('data-color-blind', p.colorBlindPalette);
  setClockFormat(p.timeFormat);
}

/** Applies the signed-in member's preferences; renders nothing. */
export function PreferencesEffects() {
  const q = usePreferences();
  useEffect(() => apply(q.data ?? DEFAULT_PREFERENCES), [q.data]);
  return null;
}

/** `/`: the member's home page, or the Log Explorer. */
export function HomeRedirect() {
  const q = usePreferences();
  if (q.isPending) return null;
  return <Navigate to={q.data?.homePage || '/logs'} replace />;
}
