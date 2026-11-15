// Tagging policy evaluation, as the server does it (server/crates/oc-api/src/routes/tagging_rules.rs): used by the
// in-browser mock. A key may list alternatives (`tf_repo|git_repo`); keys compare without
// case, `-` and `_` alike; allowed values compare without case.
import type { CoverageCount, CoverageRow, NonCompliantResource, PolicyGaps, TagCoverage, TagGap, TaggedResource, TagRule } from './types';

export const NO_TEAM = '(no team)';

const normKey = (k: string) => k.trim().toLowerCase().replace(/-/g, '_');

/** Every value of `key` among `key:value` tags. */
export function tagValues(tags: string[], key: string): string[] {
  const want = normKey(key);
  const out: string[] = [];
  for (const t of tags) {
    const i = t.indexOf(':');
    if (i < 0 || normKey(t.slice(0, i)) !== want) continue;
    const v = t.slice(i + 1).trim();
    if (v) out.push(v);
  }
  return out;
}

/** Case-insensitive `*` pattern match (oc_query::glob). */
export function glob(pattern: string, value: string): boolean {
  const re = new RegExp(
    `^${pattern
      .toLowerCase()
      .replace(/[.+?^${}()|[\]\\]/g, '\\$&')
      .replace(/\*/g, '.*')}$`,
  );
  return re.test(value.toLowerCase());
}

const patterns = (s: string | undefined) =>
  (s ?? '')
    .split(',')
    .map((p) => p.trim())
    .filter(Boolean);

function inScope(field: string | undefined, value: string | null | undefined): boolean {
  const ps = patterns(field);
  return !ps.length || (value != null && ps.some((p) => glob(p, value)));
}

/** A `key` or `key:value` pattern matches one of the `key:value` tags (keys compared as normKey). */
function labelExcluded(field: string | undefined, tags: string[]): boolean {
  return patterns(field).some((p) => {
    const i = p.indexOf(':');
    const key = normKey(i < 0 ? p : p.slice(0, i));
    const value = i < 0 ? null : p.slice(i + 1).trim();
    return tags.some((t) => {
      const j = t.indexOf(':');
      return j >= 0 && glob(key, normKey(t.slice(0, j))) && (value === null || glob(value, t.slice(j + 1).trim()));
    });
  });
}

/** The resource is of the rule's target and in its scope, and not excluded by its namespace or labels. */
export function applies(rule: TagRule, r: TaggedResource): boolean {
  const s = rule.scope ?? {};
  return (
    r.target === rule.target &&
    inScope(s.provider, r.provider) &&
    inScope(s.account, r.account) &&
    inScope(s.cluster, r.cluster) &&
    inScope(s.resourceType, r.resourceType) &&
    inScope(s.names, r.name) &&
    !(r.namespace != null && patterns(s.excludeNamespaces).some((p) => glob(p, r.namespace!))) &&
    !labelExcluded(s.excludeLabels, r.tags)
  );
}

/** The required tags the resource lacks or carries with values not allowed. */
export function gaps(rule: TagRule, r: TaggedResource): TagGap[] {
  const out: TagGap[] = [];
  for (const t of rule.required) {
    const values = t.key.split('|').flatMap((k) => tagValues(r.tags, k));
    const allowed = (t.allowedValues ?? []).filter(Boolean);
    if (!values.length) out.push({ key: t.key, value: null });
    else if (allowed.length) {
      const bad = values.find((v) => !allowed.some((a) => a.toLowerCase() === v.toLowerCase()));
      if (bad !== undefined) out.push({ key: t.key, value: bad });
    }
  }
  return out;
}

/** `tf_repo missing`, `env=production not allowed`. */
export const describeGap = (g: TagGap) => (g.value == null ? `${g.key} missing` : `${g.key}=${g.value} not allowed`);

/** Compliant share in percent with one decimal; null when nothing is evaluated. */
export function count(evaluated: number, compliant: number): CoverageCount {
  return { evaluated, compliant, score: evaluated ? Math.round((compliant * 1000) / evaluated) / 10 : null };
}

function rows(m: Map<string, [number, number]>): CoverageRow[] {
  return [...m.entries()].map(([key, [e, c]]) => ({ key, label: key, ...count(e, c) })).sort((a, b) => (a.score ?? 100) - (b.score ?? 100) || b.evaluated - a.evaluated || a.key.localeCompare(b.key));
}

export interface EvaluatedPolicy {
  id: string;
  name: string;
  rule: TagRule;
}

type Coverage = Omit<TagCoverage, 'history' | 'provenance' | 'argocdApplications'>;

/** Every policy on every resource: coverage per policy, target, account, cluster and team, and up to `limit` failing resources. */
export function coverage(policies: EvaluatedPolicy[], resources: TaggedResource[], limit: number): Coverage {
  const byPolicy = policies.map(() => [0, 0] as [number, number]);
  const groups = { target: new Map<string, [number, number]>(), account: new Map<string, [number, number]>(), cluster: new Map<string, [number, number]>(), team: new Map<string, [number, number]>() };
  const add = (m: Map<string, [number, number]>, k: string, ok: boolean) => {
    const v = m.get(k) ?? [0, 0];
    m.set(k, [v[0] + 1, v[1] + (ok ? 1 : 0)]);
  };
  let evaluated = 0;
  let compliant = 0;
  let withProvenance = 0;
  const noncompliant: NonCompliantResource[] = [];
  let noncompliantTotal = 0;
  for (const r of resources) {
    const failing: PolicyGaps[] = [];
    let applied = false;
    policies.forEach((p, i) => {
      if (!applies(p.rule, r)) return;
      applied = true;
      const g = gaps(p.rule, r);
      byPolicy[i][0]++;
      if (!g.length) byPolicy[i][1]++;
      else failing.push({ policyId: p.id, policyName: p.name, gaps: g });
    });
    if (!applied) continue;
    const ok = !failing.length;
    evaluated++;
    if (ok) compliant++;
    if (r.provenance) withProvenance++;
    add(groups.target, r.target, ok);
    if (r.account) add(groups.account, r.account, ok);
    if (r.cluster) add(groups.cluster, r.cluster, ok);
    add(groups.team, r.team ?? NO_TEAM, ok);
    if (!ok) {
      noncompliantTotal++;
      if (noncompliant.length < limit) noncompliant.push({ resource: r, policies: failing });
    }
  }
  return {
    total: count(evaluated, compliant),
    withProvenance,
    byPolicy: policies.map((p, i) => ({ key: p.id, label: p.name, ...count(byPolicy[i][0], byPolicy[i][1]) })),
    byTarget: rows(groups.target),
    byAccount: rows(groups.account),
    byCluster: rows(groups.cluster),
    byTeam: rows(groups.team),
    noncompliant,
    noncompliantTotal,
  };
}
