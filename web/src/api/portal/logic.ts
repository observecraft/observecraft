// Pure helpers of the Developer Portal home page (also exercised by scripts/portal-home.test.mjs).
import type { PortalEntity, PortalPullRequest, PrGroup } from './types';

/** Greeting by the viewer's local time of day. */
export function greeting(date: Date): string {
  const h = date.getHours();
  if (h >= 5 && h < 12) return 'Good morning';
  if (h >= 12 && h < 18) return 'Good afternoon';
  return 'Good evening';
}

/** First name from a display name, or the part of the email before the first dot or @. */
export function firstName(name: string, email = ''): string {
  const first = name.trim().split(/\s+/)[0];
  if (first) return first;
  const local = email.split('@')[0].split(/[._+-]/)[0];
  return local ? local[0].toUpperCase() + local.slice(1) : '';
}

/** "Oct 28, 2026" */
export function dateLine(date: Date): string {
  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

export const PR_GROUPS: { key: PrGroup; label: string }[] = [
  { key: 'needs_review', label: 'Needs your review' },
  { key: 'returned', label: 'Returned to you' },
  { key: 'approved', label: 'Approved' },
  { key: 'waiting', label: 'Waiting for reviewers' },
  { key: 'drafts', label: 'Drafts' },
  { key: 'merged', label: 'Recently merged' },
];

/** Every group in display order (empty ones included), newest first inside a group. */
export function groupPullRequests(prs: PortalPullRequest[]): { key: PrGroup; label: string; items: PortalPullRequest[] }[] {
  return PR_GROUPS.map((g) => ({ ...g, items: prs.filter((p) => p.group === g.key).sort((a, b) => b.updatedAt - a.updatedAt) }));
}

export type EntityTab = 'recents' | 'teams' | 'favorites';

/** Rows of a Services & Entities tab: recently active ones, those of the viewer's teams, or starred ones. */
export function entitiesOf(entities: PortalEntity[], tab: EntityTab, myTeams: string[], query = ''): PortalEntity[] {
  const q = query.trim().toLowerCase();
  const rows = entities.filter((e) => {
    if (q && !e.name.toLowerCase().includes(q) && !e.team.toLowerCase().includes(q)) return false;
    if (tab === 'teams') return !!e.team && myTeams.includes(e.team);
    if (tab === 'favorites') return e.favorite;
    return e.lastSeen != null;
  });
  return tab === 'recents' ? rows.slice(0, 10) : rows;
}

/** "3h ago" style age for table cells; "—" when unknown. */
export function age(ts: number | null | undefined, now = Date.now()): string {
  if (ts == null || ts <= 0) return '—';
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 60) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  if (d < 30) return `${d}d ago`;
  return new Date(ts).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

/** Turns the logo key a user picked or typed into a URL's host into a logo registry key. */
export function guessLogo(url: string): string {
  let host = '';
  try {
    host = new URL(url).hostname.toLowerCase();
  } catch {
    return '';
  }
  const known: [RegExp, string][] = [
    [/github/, 'github'],
    [/gitlab/, 'gitlab'],
    [/atlassian\.net$|jira/, 'jira'],
    [/confluence/, 'confluence'],
    [/linear\.app$/, 'linear'],
    [/notion\.(so|site)$/, 'notion'],
    [/figma\.com$/, 'figma'],
    [/slack\.com$/, 'slack'],
    [/pagerduty\.com$/, 'pagerduty'],
    [/sentry/, 'sentry'],
    [/bitbucket/, 'bitbucket'],
    [/launchdarkly/, 'launchdarkly'],
    [/sonar/, 'sonarqube'],
    [/drive\.google\.com$/, 'googledrive'],
    [/servicenow/, 'tile:servicenow'],
    [/dev\.azure\.com$|visualstudio\.com$/, 'tile:azure-devops'],
  ];
  return known.find(([re]) => re.test(host))?.[1] ?? '';
}
