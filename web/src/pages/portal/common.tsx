import { CheckCircle2, CircleMinus, XCircle } from 'lucide-react';
import type { ScorecardOutcomeState } from '../../api/scorecards/client';
import type { Counts } from '../../api/scorecards/logic';

/** A level chip: L1 light, L2 medium, L3 dark (L0: not yet at level 1). */
export function LevelBadge({ level, prefix = 'L' }: { level: number; prefix?: string }) {
  return (
    <span className={`sc-level sc-level-${Math.max(0, Math.min(3, level))}`} title={level ? `Level ${level}` : 'Below level 1'}>
      {prefix}
      {level}
    </span>
  );
}

export function StateIcon({ state, size = 15 }: { state: ScorecardOutcomeState; size?: number }) {
  if (state === 'pass') return <CheckCircle2 size={size} className="sc-state sc-state-pass" aria-label="Pass" />;
  if (state === 'fail') return <XCircle size={size} className="sc-state sc-state-fail" aria-label="Fail" />;
  return <CircleMinus size={size} className="sc-state sc-state-skip" aria-label="Skip" />;
}

/** Pass / fail / skip shares as one thin stacked bar. */
export function PassBar({ counts, wide = false }: { counts: Counts; wide?: boolean }) {
  const pct = (n: number) => (counts.total ? (n * 100) / counts.total : 0);
  return (
    <span className={`sc-passbar${wide ? ' wide' : ''}`} title={`${counts.pass} pass · ${counts.fail} fail · ${counts.skip} skip`}>
      <span className="sc-passbar-pass" style={{ width: `${pct(counts.pass)}%` }} />
      <span className="sc-passbar-fail" style={{ width: `${pct(counts.fail)}%` }} />
      <span className="sc-passbar-skip" style={{ width: `${pct(counts.skip)}%` }} />
    </span>
  );
}

export const fmtPct = (v: number | null | undefined) => (v === null || v === undefined ? '—' : `${Math.round(v)}%`);

/** Level distribution (L0–L3) as a stacked bar. */
export function LevelBar({ levels }: { levels: number[] }) {
  const total = levels.reduce((a, b) => a + b, 0);
  return (
    <span className="sc-levelbar" title={levels.map((n, l) => `L${l}: ${n}`).join(' · ')}>
      {levels.map((n, l) => (n ? <span key={l} className={`sc-level-${l}`} style={{ width: `${(n * 100) / total}%` }} /> : null))}
    </span>
  );
}
