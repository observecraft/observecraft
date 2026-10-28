import { useState } from 'react';
import { ArrowRight } from 'lucide-react';
import type { ScorecardRecommendation } from '../../api/scorecards/client';
import { recommendedGroups } from '../../api/scorecards/logic';
import { LevelBadge } from './common';
import { useSetupScorecards } from './queries';

/** First run: the hero band with the recommended scorecards to pick from, and what scorecards do. */
export function ScorecardsIntro({ recommended }: { recommended: ScorecardRecommendation[] }) {
  const [picked, setPicked] = useState<Set<string>>(() => new Set(recommended.map((r) => r.key)));
  const setup = useSetupScorecards();
  const groups = recommendedGroups(recommended);
  const toggle = (key: string) =>
    setPicked((p) => {
      const next = new Set(p);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  return (
    <div className="sc-intro">
      <section className="sc-hero">
        <div className="sc-hero-copy">
          <h1>Raise every service to production grade with scorecards</h1>
          <p>Rules check each service's telemetry, ownership and readiness, so teams see what to fix next and how far they have come.</p>
        </div>
        <HeroArt />
        <div className="sc-start-card">
          <h2>Get started with recommended scorecards</h2>
          <p className="sc-start-lede">Pick the rules to start with. You can change levels, scopes and rules later.</p>
          <div className="sc-start-head">
            <span>Rule</span>
            <span>Level</span>
          </div>
          <div className="sc-start-groups">
            {groups.map((g) => (
              <div key={g.scorecard} className="sc-start-group">
                <div className="sc-start-group-name">{g.scorecard}</div>
                {g.rules.map((r) => (
                  <label key={r.key} className="sc-start-rule" title={r.description}>
                    <input type="checkbox" checked={picked.has(r.key)} onChange={() => toggle(r.key)} />
                    <span className="sc-start-rule-name">{r.name}</span>
                    <LevelBadge level={r.level} prefix="" />
                  </label>
                ))}
              </div>
            ))}
          </div>
          {setup.isError && <div className="form-error">{(setup.error as Error).message}</div>}
          <button className="btn btn-primary sc-start-button" disabled={setup.isPending} onClick={() => setup.mutate([...picked])}>
            {setup.isPending ? 'Creating scorecards…' : 'Create Your Scorecards'} <ArrowRight size={15} />
          </button>
        </div>
      </section>

      <section className="sc-features">
        <Feature title="Identify gaps with out-of-the-box rules" art={<RulesArt />}>
          Built-in rules read what ObserveCraft already has: versions on spans, trace ids on logs, monitors, SLOs, deployments and the owners you record for each service. Every service gets a pass,
          fail or skip with the reason.
        </Feature>
        <Feature title="Level up one step at a time" art={<LevelsArt />}>
          Rules carry a level from 1 (basics) to 3 (advanced). A service reaches a level when nothing at or below it fails, so teams know which fixes come first.
        </Feature>
        <Feature title="Track progress across teams" art={<TeamsArt />}>
          Roll scores up by team and scorecard, evaluate on demand or let the hourly evaluation keep them fresh, and add your own rules with outcomes reported from scripts or CI.
        </Feature>
      </section>
    </div>
  );
}

function Feature({ title, art, children }: { title: string; art: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="sc-feature">
      <h3>{title}</h3>
      <p>{children}</p>
      <div className="sc-feature-art">{art}</div>
    </div>
  );
}

/** Decorative rings and a rising bar trio behind the headline. */
function HeroArt() {
  return (
    <svg className="sc-hero-art" viewBox="0 0 520 260" aria-hidden="true">
      <circle cx="420" cy="40" r="150" className="sc-hero-ring" />
      <circle cx="420" cy="40" r="100" className="sc-hero-ring" />
      <circle cx="420" cy="40" r="52" className="sc-hero-ring" />
      <rect x="40" y="170" width="34" height="50" rx="4" className="sc-hero-bar" />
      <rect x="86" y="130" width="34" height="90" rx="4" className="sc-hero-bar" />
      <rect x="132" y="80" width="34" height="140" rx="4" className="sc-hero-bar strong" />
    </svg>
  );
}

/** A miniature rules table with pass and fail marks. */
function RulesArt() {
  const rows: [string, number, boolean[]][] = [
    ['Monitors defined', 1, [true, true, false, true]],
    ['SLOs defined', 2, [true, false, false, true]],
    ['Logs correlation', 3, [true, true, true, false]],
  ];
  return (
    <svg viewBox="0 0 260 120" aria-hidden="true">
      <rect x="1" y="1" width="258" height="118" rx="6" className="sc-art-card" />
      <rect x="1" y="1" width="258" height="22" rx="6" className="sc-art-head" />
      {rows.map(([name, level, marks], i) => {
        const y = 38 + i * 28;
        return (
          <g key={name}>
            <text x="12" y={y + 4} className="sc-art-text">
              {name}
            </text>
            <rect x="118" y={y - 8} width="22" height="14" rx="3" className={`sc-art-level-${level}`} />
            <text x="129" y={y + 3} textAnchor="middle" className={`sc-art-level-text-${level}`}>
              L{level}
            </text>
            {marks.map((ok, j) => (
              <circle key={j} cx={164 + j * 22} cy={y - 1} r="6" className={ok ? 'sc-art-pass' : 'sc-art-fail'} />
            ))}
          </g>
        );
      })}
    </svg>
  );
}

/** Three stacked steps, the top one highlighted. */
function LevelsArt() {
  return (
    <svg viewBox="0 0 260 120" aria-hidden="true">
      {[0, 1, 2].map((i) => (
        <g key={i}>
          <rect x={40 + i * 60} y={84 - i * 28} width="60" height={28 + i * 28} rx="4" className={`sc-art-level-${i + 1}`} />
          <text x={70 + i * 60} y={102 - i * 28 + i * 0} textAnchor="middle" className={`sc-art-level-text-${i + 1}`}>
            L{i + 1}
          </text>
        </g>
      ))}
      <path d="M58 52 L120 30 L184 14" className="sc-art-arrow" />
      <circle cx="184" cy="14" r="4" className="sc-art-pass" />
    </svg>
  );
}

/** Per-team score bars. */
function TeamsArt() {
  const teams: [string, number][] = [
    ['checkout', 0.82],
    ['storefront', 0.64],
    ['catalog', 0.45],
    ['billing', 0.3],
  ];
  return (
    <svg viewBox="0 0 260 120" aria-hidden="true">
      <rect x="1" y="1" width="258" height="118" rx="6" className="sc-art-card" />
      {teams.map(([name, v], i) => {
        const y = 22 + i * 24;
        return (
          <g key={name}>
            <text x="12" y={y + 4} className="sc-art-text">
              {name}
            </text>
            <rect x="86" y={y - 6} width="150" height="10" rx="5" className="sc-art-track" />
            <rect x="86" y={y - 6} width={150 * v} height="10" rx="5" className={v > 0.6 ? 'sc-art-bar-good' : 'sc-art-bar'} />
          </g>
        );
      })}
    </svg>
  );
}
