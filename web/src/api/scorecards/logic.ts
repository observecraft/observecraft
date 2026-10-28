// Scorecards: levels, scores, rule summaries and team rollups shared by the pages and the in-browser
// mock, plus the built-in rule checks the mock runs. They follow the server
// (routes/scorecard_rules.rs and routes/scorecards.rs).
import type { ScorecardOutcome, ScorecardOutcomeState, ScorecardRecommendation, ScorecardRule, ScorecardService } from './types';

export const DAY = 86_400_000;
export const MAX_LEVEL = 3;
export const LEVELS = [1, 2, 3] as const;

/** The recommended scorecards, in the order the first-run card lists them. */
export const BUILTIN_SCORECARDS = ['Observability Best Practices', 'Ownership & Documentation', 'Production Readiness'];

export const STATE_LABEL: Record<ScorecardOutcomeState, string> = { pass: 'Pass', fail: 'Fail', skip: 'Skip' };

type Leveled = { level: number; state: ScorecardOutcomeState };

/** Highest level up to which no rule fails; 0 when a level-1 rule fails or nothing passes. Skips don't count. */
export function serviceLevel(outcomes: Leveled[]): number {
  if (!outcomes.some((o) => o.state === 'pass')) return 0;
  let level = 0;
  for (const l of LEVELS) {
    if (outcomes.some((o) => o.level === l && o.state === 'fail')) break;
    level = l;
  }
  return level;
}

/** Passing share of passed and failed rules, in percent; null when nothing was evaluated. */
export function score(outcomes: Leveled[]): number | null {
  const pass = outcomes.filter((o) => o.state === 'pass').length;
  const fail = outcomes.filter((o) => o.state === 'fail').length;
  return pass + fail ? (pass * 100) / (pass + fail) : null;
}

const nameChar = (c: string | undefined) => !!c && /[\p{L}\p{N}\-_./]/u.test(c);

/** Whether a query or tag mentions `service:<name>` (quoted, or in a `service:(a OR b)` list), not a longer name. */
export function referencesService(text: string, service: string): boolean {
  const t = text.toLowerCase();
  const s = service.toLowerCase();
  let from = 0;
  for (;;) {
    const at = t.indexOf('service:', from);
    if (at < 0) return false;
    from = at + 'service:'.length;
    const before = t[at - 1];
    if (nameChar(before) || before === '@') continue;
    const rest = t.slice(from);
    let values: string[];
    if (rest.startsWith('(')) {
      const end = rest.indexOf(')');
      values = rest
        .slice(1, end < 0 ? undefined : end)
        .split(/[\s,]+/)
        .filter((v) => v && v !== 'or')
        .map((v) => v.replace(/^"|"$/g, ''));
    } else if (rest.startsWith('"')) {
      const end = rest.indexOf('"', 1);
      values = [rest.slice(1, end < 0 ? undefined : end)];
    } else {
      let end = 0;
      while (end < rest.length && nameChar(rest[end])) end++;
      values = [rest.slice(0, end)];
    }
    if (values.includes(s)) return true;
  }
}

export interface Counts {
  pass: number;
  fail: number;
  skip: number;
  total: number;
  /** Passing share of passed and failed outcomes (percent); null when none. */
  passPct: number | null;
}

export function countStates(outcomes: { state: ScorecardOutcomeState }[]): Counts {
  const pass = outcomes.filter((o) => o.state === 'pass').length;
  const fail = outcomes.filter((o) => o.state === 'fail').length;
  const skip = outcomes.length - pass - fail;
  return { pass, fail, skip, total: outcomes.length, passPct: pass + fail ? (pass * 100) / (pass + fail) : null };
}

export type SummaryBy = 'entity' | 'kind' | 'team';

export interface SummaryGroup extends Counts {
  key: string;
  label: string;
  outcomes: ScorecardOutcome[];
}

/** A rule's outcomes grouped per service, entity kind or owning team (failing groups first). */
export function summarize(outcomes: ScorecardOutcome[], services: ScorecardService[], by: SummaryBy): SummaryGroup[] {
  const team = new Map(services.map((s) => [s.service, s.team]));
  const keyOf = (o: ScorecardOutcome) => (by === 'entity' ? o.service : by === 'kind' ? 'service' : team.get(o.service) || '');
  const groups = new Map<string, ScorecardOutcome[]>();
  for (const o of outcomes) groups.set(keyOf(o), [...(groups.get(keyOf(o)) ?? []), o]);
  const label = (k: string) => (by === 'team' && !k ? 'No team' : k);
  return [...groups].map(([key, list]) => ({ key, label: label(key), outcomes: list, ...countStates(list) })).sort((a, b) => b.fail - a.fail || a.label.localeCompare(b.label));
}

export interface TeamRollup {
  /** Team handle; '' for services without a team. */
  team: string;
  services: number;
  /** Average of the services' scores (percent); null when none is scored. */
  avgScore: number | null;
  /** Services per level 0–3. */
  levels: number[];
}

/** Per-team averages and level counts, teams with the lowest average first; services without a team last. */
export function teamRollup(services: ScorecardService[]): TeamRollup[] {
  const by = new Map<string, ScorecardService[]>();
  for (const s of services) by.set(s.team, [...(by.get(s.team) ?? []), s]);
  return [...by]
    .map(([team, list]) => {
      const scored = list.map((s) => s.score).filter((s): s is number => s !== null && s !== undefined);
      return { team, services: list.length, avgScore: scored.length ? scored.reduce((a, b) => a + b, 0) / scored.length : null, levels: levelCounts(list) };
    })
    .sort((a, b) => Number(!a.team) - Number(!b.team) || (a.avgScore ?? 101) - (b.avgScore ?? 101) || a.team.localeCompare(b.team));
}

/** Number of services at each level 0–3. */
export function levelCounts(services: { level: number }[]): number[] {
  const out = [0, 0, 0, 0];
  for (const s of services) out[Math.min(MAX_LEVEL, Math.max(0, s.level))]++;
  return out;
}

/** Recommended rules grouped by scorecard in display order, each group by level then name. */
export function recommendedGroups(recommended: ScorecardRecommendation[]): { scorecard: string; rules: ScorecardRecommendation[] }[] {
  const names = [...BUILTIN_SCORECARDS, ...recommended.map((r) => r.scorecard).filter((n) => !BUILTIN_SCORECARDS.includes(n))];
  return [...new Set(names)]
    .map((scorecard) => ({ scorecard, rules: recommended.filter((r) => r.scorecard === scorecard).sort((a, b) => a.name.localeCompare(b.name)) }))
    .filter((g) => g.rules.length);
}

/** Outcomes of one service, with each rule. */
export function serviceOutcomes(service: string, rules: ScorecardRule[], outcomes: ScorecardOutcome[]): { rule: ScorecardRule; outcome: ScorecardOutcome }[] {
  const byId = new Map(rules.map((r) => [r.id, r]));
  return outcomes.filter((o) => o.service === service && byId.has(o.ruleId)).map((o) => ({ rule: byId.get(o.ruleId)!, outcome: o }));
}

// ---------------------------------------------------------------- built-in checks (mock)

export interface VersionSeen {
  /** '' for spans without service.version. */
  version: string;
  firstSeen: number;
  lastSeen: number;
  /** Spans of the past 7 days. */
  recentSpans: number;
}

export interface ServiceFacts {
  team: string;
  contacts: number;
  repoUrl: string;
  docsUrl: string;
  versions: VersionSeen[];
  logs: number;
  tracedLogs: number;
  lastCiDeploy: number | null;
  monitors: string[];
  slos: string[];
}

export interface Verdict {
  state: ScorecardOutcomeState;
  reason: string;
}

const pass = (reason: string): Verdict => ({ state: 'pass', reason });
const fail = (reason: string): Verdict => ({ state: 'fail', reason });

export function daysAgo(now: number, t: number): string {
  const n = Math.floor(Math.max(0, now - t) / DAY);
  return n === 0 ? 'today' : n === 1 ? '1 day ago' : `${n} days ago`;
}

const latest = (versions: VersionSeen[]) => versions.filter((v) => v.version).sort((a, b) => b.lastSeen - a.lastSeen)[0];

function references(names: string[], one: string, many: string): Verdict {
  if (!names.length) return fail(`No ${one} references this service (service:<name> in its query or tags)`);
  const list = names.slice(0, 3).join(', ') + (names.length > 3 ? ` and ${names.length - 3} more` : '');
  return pass(`${names.length} ${names.length === 1 ? one : many}: ${list}`);
}

function deployedRecently(f: ServiceFacts, now: number): Verdict {
  const nothing = 'No spans with service.version and no CI deployment in the past 90 days';
  const windowStart = now - 90 * DAY;
  if (f.lastCiDeploy !== null && f.lastCiDeploy >= windowStart) return pass(`Deployed by CI ${daysAgo(now, f.lastCiDeploy)}`);
  if (!f.versions.length) return fail(nothing);
  const earliest = Math.min(...f.versions.map((v) => v.firstSeen));
  const fresh = f.versions.filter((v) => v.version && v.firstSeen > earliest + DAY && v.firstSeen >= windowStart).sort((a, b) => b.firstSeen - a.firstSeen)[0];
  if (fresh) return pass(`Version ${fresh.version} first seen ${daysAgo(now, fresh.firstSeen)}`);
  const v = latest(f.versions);
  if (!v) return fail(nothing);
  return earliest <= windowStart + DAY
    ? fail(`Version ${v.version} has been running for more than 90 days`)
    : fail(`Only version ${v.version} seen since ${daysAgo(now, earliest)}, as far back as spans are kept`);
}

/** A built-in rule's outcome for one service (`onCallMissing`: why the on-call rule skips). */
export function evaluateBuiltin(key: string, f: ServiceFacts, now: number, onCallMissing = 'On-Call is not configured'): Verdict {
  const defined = (value: string, what: string) => (value.trim() ? pass(`${what[0].toUpperCase()}${what.slice(1)}: ${value.trim()}`) : fail(`No ${what} in the service's metadata`));
  switch (key) {
    case 'deployment_tracking': {
      const recent = f.versions.filter((v) => v.recentSpans > 0);
      if (!recent.length) return fail('No spans in the past 7 days');
      const v = latest(recent);
      return v ? pass(`Spans carry service.version (latest ${v.version})`) : fail("Spans of the past 7 days don't carry service.version");
    }
    case 'logs_correlation':
      if (!f.logs) return fail('No logs in the past 7 days');
      if (!f.tracedLogs) return fail('No log of the past 7 days carries a trace id');
      return pass(`${Math.round((f.tracedLogs * 100) / f.logs)}% of the logs of the past 7 days carry a trace id`);
    case 'team_defined':
      return f.team.trim() ? pass(`Owned by team ${f.team.trim()}`) : fail("No team in the service's metadata");
    case 'contacts_defined':
      return f.contacts ? pass(f.contacts === 1 ? '1 contact' : `${f.contacts} contacts`) : fail("No contacts in the service's metadata");
    case 'code_repos_defined':
      return defined(f.repoUrl, 'code repository');
    case 'docs_defined':
      return defined(f.docsUrl, 'documentation link');
    case 'deployed_recently':
      return deployedRecently(f, now);
    case 'monitors_defined':
      return references(f.monitors, 'monitor', 'monitors');
    case 'slos_defined':
      return references(f.slos, 'SLO', 'SLOs');
    case 'on_call_defined':
      return { state: 'skip', reason: onCallMissing };
    default:
      return { state: 'skip', reason: 'Unknown rule' };
  }
}
