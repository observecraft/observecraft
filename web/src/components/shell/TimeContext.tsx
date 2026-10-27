import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import type { TimeRange } from '../../api/types';

export interface RelativeRange {
  key: string;
  short: string;
  label: string;
  ms: number;
}

export const RELATIVE_RANGES: RelativeRange[] = [
  { key: '15m', short: '15m', label: 'Past 15 Minutes', ms: 15 * 60e3 },
  { key: '1h', short: '1h', label: 'Past 1 Hour', ms: 3600e3 },
  { key: '4h', short: '4h', label: 'Past 4 Hours', ms: 4 * 3600e3 },
  { key: '1d', short: '1d', label: 'Past 1 Day', ms: 24 * 3600e3 },
  { key: '2d', short: '2d', label: 'Past 2 Days', ms: 2 * 24 * 3600e3 },
  { key: '1w', short: '1w', label: 'Past 7 Days', ms: 7 * 24 * 3600e3 },
];

interface TimeState {
  relative: RelativeRange;
  range: TimeRange;
  live: boolean;
  setRelative: (key: string) => void;
  setLive: (live: boolean) => void;
  shift: (direction: -1 | 1) => void;
  zoomOut: () => void;
  refresh: () => void;
  refreshVersion: number;
}

const Ctx = createContext<TimeState | null>(null);

/**
 * Time selection behind the top-right time picker. The app has one global provider;
 * a section (e.g. Error Tracking, a side panel) can nest its own with a different default.
 */
export function TimeProvider({ children, initial = '1h' }: { children: ReactNode; initial?: string }) {
  const [relative, setRel] = useState(() => RELATIVE_RANGES.find((r) => r.key === initial) ?? RELATIVE_RANGES[1]);
  const [to, setTo] = useState(() => Date.now());
  const [live, setLive] = useState(true);
  const [refreshVersion, setRefreshVersion] = useState(0);

  // Live mode slides the window forward every 10s.
  useEffect(() => {
    if (!live) return;
    const id = setInterval(() => setTo(Date.now()), 10_000);
    return () => clearInterval(id);
  }, [live]);

  const setRelative = useCallback((key: string) => {
    const r = RELATIVE_RANGES.find((x) => x.key === key);
    if (r) {
      setRel(r);
      setTo(Date.now());
    }
  }, []);

  const shift = useCallback(
    (dir: -1 | 1) => {
      setLive(false);
      setTo((t) => Math.min(Date.now(), t + (dir * relative.ms) / 2));
    },
    [relative],
  );

  const zoomOut = useCallback(() => {
    const idx = RELATIVE_RANGES.indexOf(relative);
    setRel(RELATIVE_RANGES[Math.min(idx + 1, RELATIVE_RANGES.length - 1)]);
  }, [relative]);

  const value = useMemo<TimeState>(
    () => ({
      relative,
      range: { from: to - relative.ms, to },
      live,
      setRelative,
      setLive,
      shift,
      zoomOut,
      refresh: () => {
        setTo(live ? Date.now() : to);
        setRefreshVersion((v) => v + 1);
      },
      refreshVersion,
    }),
    [relative, to, live, refreshVersion, setRelative, shift, zoomOut],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useTime(): TimeState {
  const v = useContext(Ctx);
  if (!v) throw new Error('useTime must be used inside TimeProvider');
  return v;
}
