// Developer Portal → Reports aggregations, following routes/portal_reports.rs (used by the mock),
// plus the ones the UI builds itself: Catalog Health and Reliability Overview.
import type { MonitorItem } from '../monitors/client';
import type { SloItem } from '../slos/types';
import type { PortalEntity, PortalScorecardTeam, PortalTeamReport } from './types';

export const DAY = 86_400_000;

export interface IncidentFact {
  declaredAt: number;
  resolvedAt: number | null;
  open: boolean;
  teams: string[];
  services: string[];
}

export interface CoverageFacts {
  /** service → owning team. */
  owners: Map<string, string>;
  sloTeams: string[][];
  monitorTeams: string[][];
  scheduleTeams: string[];
}

export const bucketSize = (range: { from: number; to: number }) => (range.to - range.from > 31 * DAY ? 7 * DAY : DAY);

export function buckets(range: { from: number; to: number }, size: number): number[] {
  const out: number[] = [];
  for (let b = Math.floor(range.from / size) * size; b <= range.to; b += size) out.push(b);
  return out;
}

export function teamsReport(range: { from: number; to: number }, teams: { handle: string; name: string; members: number }[], incidents: IncidentFact[], cov: CoverageFacts) {
  const size = bucketSize(range);
  const starts = buckets(range, size);
  const first = starts[0] ?? range.from;
  const owners = [...cov.owners.values()];
  const out: PortalTeamReport[] = teams.map((t) => ({
    handle: t.handle,
    name: t.name,
    members: t.members,
    incidents: starts.map(() => 0),
    incidentCount: 0,
    openIncidents: 0,
    mttrMinutes: null,
    services: owners.filter((o) => o === t.handle).length,
    slos: cov.sloTeams.filter((ts) => ts.includes(t.handle)).length,
    monitors: cov.monitorTeams.filter((ts) => ts.includes(t.handle)).length,
    onCallSchedules: cov.scheduleTeams.filter((h) => h === t.handle).length,
  }));
  const index = new Map(out.map((t, i) => [t.handle, i]));
  const repair = out.map(() => ({ sum: 0, n: 0 }));
  for (const inc of incidents) {
    const inRange = inc.declaredAt >= range.from && inc.declaredAt <= range.to;
    const teamsOf = new Set([...inc.teams, ...inc.services.map((s) => cov.owners.get(s) ?? '')].filter(Boolean));
    for (const team of teamsOf) {
      const i = index.get(team);
      if (i === undefined) continue;
      if (inc.open) out[i].openIncidents++;
      if (inRange) {
        out[i].incidentCount++;
        const b = Math.floor((inc.declaredAt - first) / size);
        if (b >= 0 && b < starts.length) out[i].incidents[b]++;
      }
      if (inc.resolvedAt !== null && inc.resolvedAt >= range.from && inc.resolvedAt <= range.to && inc.resolvedAt >= inc.declaredAt) {
        repair[i].sum += (inc.resolvedAt - inc.declaredAt) / 60_000;
        repair[i].n++;
      }
    }
  }
  out.forEach((t, i) => (t.mttrMinutes = repair[i].n ? Math.round((repair[i].sum / repair[i].n) * 10) / 10 : null));
  return { buckets: starts, bucketMs: size, teams: out };
}

/** Rows of (day start, service, team, level) → mean level per team and day, and the last day's distribution. */
export function scorecardTrend(range: { from: number; to: number }, rows: [number, string, string, number][], teams: string[]): { days: number[]; teams: PortalScorecardTeam[] } {
  const days = buckets(range, DAY);
  const index = new Map(days.map((d, i) => [d, i]));
  const sums = new Map<string, { sum: number; n: number }[]>();
  const last = new Map<string, number>();
  for (const [day, , team, level] of rows) {
    const i = index.get(day);
    if (i === undefined || (teams.length && !teams.includes(team))) continue;
    if (!sums.has(team))
      sums.set(
        team,
        days.map(() => ({ sum: 0, n: 0 })),
      );
    sums.get(team)![i].sum += level;
    sums.get(team)![i].n++;
    last.set(team, Math.max(last.get(team) ?? day, day));
  }
  const out = [...sums.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([team, s]) => {
      const current = [0, 0, 0, 0];
      for (const [day, , t, level] of rows) if (t === team && day === last.get(team)) current[Math.max(0, Math.min(3, level))]++;
      return { team, levels: s.map((x) => (x.n ? Math.round((x.sum / x.n) * 100) / 100 : null)), current };
    });
  return { days, teams: out };
}

// ================================================================ built in the UI

export interface HealthRow {
  key: 'owner' | 'docs' | 'repo' | 'contacts' | 'onCall';
  label: string;
  covered: number;
  total: number;
}

/**
 * Catalog Health over services and the entities users define (endpoints, repositories and
 * telemetry-only datastores, queues and providers are left out): owner, docs, repository,
 * contacts, and On-Call through the owning team's schedules.
 */
export function catalogHealth(entities: PortalEntity[], teamsWithOnCall: Set<string>, teams: string[] = []): { rows: HealthRow[]; entities: PortalEntity[] } {
  const scoped = entities.filter((e) => (e.kind === 'service' || !!e.definitionId) && (!teams.length || teams.includes(e.owner)));
  const count = (f: (e: PortalEntity) => boolean) => scoped.filter(f).length;
  const total = scoped.length;
  return {
    entities: scoped,
    rows: [
      { key: 'owner', label: 'Owner (team)', covered: count((e) => !!e.owner), total },
      { key: 'docs', label: 'Documentation', covered: count((e) => !!e.docsUrl || e.links.some((l) => l.type === 'doc' || l.type === 'runbook')), total },
      { key: 'repo', label: 'Code repository', covered: count((e) => !!e.repoUrl), total },
      { key: 'contacts', label: 'Contacts', covered: count((e) => e.contacts.length > 0), total },
      { key: 'onCall', label: 'On-Call', covered: count((e) => !!e.owner && teamsWithOnCall.has(e.owner)), total },
    ],
  };
}

/** The `service:` values an SLO or monitor names in its tags and queries. */
export function servicesNamed(texts: (string | null | undefined)[]): string[] {
  const out = new Set<string>();
  for (const t of texts) for (const m of (t ?? '').matchAll(/(?:^|[\s{,(])service:"?([A-Za-z0-9._\-/]+)/g)) out.add(m[1]);
  return [...out].sort();
}

export interface ReliabilityTeamRow {
  team: string;
  slos: number;
  breached: number;
  warning: number;
  /** Lowest remaining error budget across the team's SLOs, in percent. */
  minBudget: number | null;
  monitors: number;
  alerting: number;
}

/** SLOs and monitors per owning team: their own teams, else the owners of the services they name. */
export function reliabilityByTeam(slos: SloItem[], monitors: MonitorItem[], owners: Map<string, string>, teamFilter: string[] = []): ReliabilityTeamRow[] {
  const rows = new Map<string, ReliabilityTeamRow>();
  const row = (team: string) => {
    let r = rows.get(team);
    if (!r) rows.set(team, (r = { team, slos: 0, breached: 0, warning: 0, minBudget: null, monitors: 0, alerting: 0 }));
    return r;
  };
  const teamsOf = (own: string[], texts: (string | null | undefined)[]) => {
    const declared = own.map((t) => t.replace(/^@/, ''));
    const ts = declared.length ? declared : servicesNamed(texts).map((s) => owners.get(s) ?? '');
    const uniqTeams = [...new Set(ts.map((t) => t || ''))];
    return (uniqTeams.length ? uniqTeams : ['']).filter((t) => !teamFilter.length || teamFilter.includes(t));
  };
  for (const s of slos) {
    for (const team of teamsOf(s.teams, [s.goodQuery, s.totalQuery, s.sliceQuery, ...s.tags])) {
      const r = row(team);
      r.slos++;
      if (s.status.state === 'breached') r.breached++;
      if (s.status.state === 'warning') r.warning++;
      const budget = s.status.budgetRemaining;
      if (budget !== null && budget !== undefined) r.minBudget = r.minBudget === null ? budget : Math.min(r.minBudget, budget);
    }
  }
  for (const m of monitors) {
    for (const team of teamsOf(m.teams, [m.query, ...m.tags])) {
      const r = row(team);
      r.monitors++;
      if (m.status === 'alert') r.alerting++;
    }
  }
  return [...rows.values()].sort((a, b) => (a.team === '' ? 1 : b.team === '' ? -1 : a.team.localeCompare(b.team)));
}
