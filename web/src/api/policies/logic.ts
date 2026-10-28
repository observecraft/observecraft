// Resource policy evaluation, as the server does it (server/crates/oc-api/src/routes/resource_policy_rules.rs):
// used by the in-browser mock and by the editor's labels. Positive conditions must hold for every value
// of the attribute (and fail when it is missing); negative ones fail when any value matches.
import type { CatalogResource } from '../catalog/client';
import { matches, values } from '../mock/catalog';
import { parseQuery, type Term } from '../../lib/searchQuery';
import type { PolicyCondition, PolicyEvaluation, PolicyOperator, PolicyTarget } from './types';

/** `v1.31.2` → [1, 31, 2], `1.29.3-eks-a1b2` → [1, 29, 3]; null when not a version. */
export function parseVersion(s: string): number[] | null {
  const core = s
    .trim()
    .replace(/^[vV]+/, '')
    .split(/[^0-9.]/)[0];
  const parts = core.split('.').filter(Boolean);
  if (!parts.length || parts.some((p) => !/^\d+$/.test(p))) return null;
  return parts.map(Number);
}

/** -1 / 0 / 1, missing components counting as 0; null when either is not a version. */
export function compareVersions(a: string, b: string): number | null {
  const x = parseVersion(a);
  const y = parseVersion(b);
  if (!x || !y) return null;
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d) return Math.sign(d);
  }
  return 0;
}

const NEGATIVE: PolicyOperator[] = ['not_exists', 'not_equals', 'not_one_of'];

export const OPERATOR_LABELS: Record<PolicyOperator, string> = {
  exists: 'is set',
  not_exists: 'is not set',
  equals: 'equals',
  not_equals: 'does not equal',
  one_of: 'is one of',
  not_one_of: 'is not one of',
  matches: 'matches regex',
  gt: '>',
  gte: '≥',
  lt: '<',
  lte: '≤',
  version_gt: 'version >',
  version_gte: 'version ≥',
  version_lt: 'version <',
  version_lte: 'version ≤',
};

/** How many values an operator takes. */
export function operatorArity(op: PolicyOperator): 'none' | 'one' | 'many' {
  if (op === 'exists' || op === 'not_exists') return 'none';
  if (op === 'one_of' || op === 'not_one_of') return 'many';
  return 'one';
}

/** `replicas ≥ 2`, `team is set`. */
export function describeCondition(c: PolicyCondition): string {
  const v = c.values ?? [];
  return v.length ? `${c.attribute} ${OPERATOR_LABELS[c.operator]} ${v.join(', ')}` : `${c.attribute} ${OPERATOR_LABELS[c.operator]}`;
}

/** Why the condition is not met by the attribute's values, or null when it is. Throws on an invalid condition. */
export function conditionFailure(c: PolicyCondition, vals: string[]): string | null {
  const want = (c.values ?? [])[0] ?? '';
  const re = c.operator === 'matches' ? new RegExp(want) : null;
  const ord = (o: number | null) => {
    if (o == null) return false;
    if (c.operator.endsWith('gte')) return o >= 0;
    if (c.operator.endsWith('gt')) return o > 0;
    if (c.operator.endsWith('lte')) return o <= 0;
    return o < 0;
  };
  const ok = (v: string): boolean => {
    switch (c.operator) {
      case 'exists':
      case 'not_exists':
        return true;
      case 'equals':
      case 'not_equals':
        return v.toLowerCase() === want.toLowerCase();
      case 'one_of':
      case 'not_one_of':
        return (c.values ?? []).some((w) => w.toLowerCase() === v.toLowerCase());
      case 'matches':
        return re!.test(v);
      case 'gt':
      case 'gte':
      case 'lt':
      case 'lte': {
        const n = Number(v);
        return v.trim() !== '' && Number.isFinite(n) && ord(Math.sign(n - Number(want)));
      }
      default:
        return ord(compareVersions(v, want));
    }
  };
  if (NEGATIVE.includes(c.operator)) {
    const hits = vals.filter(ok);
    return hits.length ? `${c.attribute} is ${(c.operator === 'not_exists' ? vals : hits).join(', ')}` : null;
  }
  if (!vals.length) return `${c.attribute} is missing`;
  return vals.every(ok) ? null : `${c.attribute} is ${vals.join(', ')}`;
}

/** The first problem with a policy target, or null when it can be evaluated. */
export function validateTarget(t: PolicyTarget): string | null {
  if (!t.conditions.length) return 'A policy needs at least one condition';
  for (const c of t.conditions) {
    const attr = c.attribute.trim();
    if (!attr || !/^[\p{L}\p{N}_.\-/@]+$/u.test(attr)) return `'${attr}' is not a valid attribute name`;
    const vals = (c.values ?? []).map((v) => v.trim()).filter(Boolean);
    const arity = operatorArity(c.operator);
    if (arity === 'none' && vals.length) return `${attr}: '${OPERATOR_LABELS[c.operator]}' takes no value`;
    if (arity === 'one' && vals.length !== 1) return `${attr}: '${OPERATOR_LABELS[c.operator]}' takes one value`;
    if (arity === 'many' && !vals.length) return `${attr}: '${OPERATOR_LABELS[c.operator]}' takes 1 to 50 values`;
    if (c.operator === 'matches') {
      try {
        new RegExp(vals[0]);
      } catch {
        return `${attr}: invalid pattern`;
      }
    }
    if (['gt', 'gte', 'lt', 'lte'].includes(c.operator) && !Number.isFinite(Number(vals[0]))) return `${attr}: '${vals[0]}' is not a number`;
    if (c.operator.startsWith('version_') && !parseVersion(vals[0])) return `${attr}: '${vals[0]}' is not a version`;
  }
  return null;
}

/** Compliant share in percent with one decimal; null when nothing is evaluated. */
export function score(evaluated: number, noncompliant: number): number | null {
  return evaluated > 0 ? Math.round(((evaluated - Math.min(noncompliant, evaluated)) * 1000) / evaluated) / 10 : null;
}

export interface Violation {
  resource: CatalogResource;
  reason: string;
}

/** Evaluates a policy on catalog rows limited to `filter` (the page's header filters). */
export function evaluate(t: PolicyTarget, resources: CatalogResource[], filter: Term[] = []): { evaluation: PolicyEvaluation; violations: Violation[] } {
  const scope = parseQuery(t.scope ?? '');
  const conditions = t.conditions.map((c) => ({ ...c, attribute: c.attribute.trim(), values: (c.values ?? []).map((v) => v.trim()).filter(Boolean) }));
  const violations: Violation[] = [];
  let evaluated = 0;
  for (const r of resources) {
    if (r.resourceType !== t.resourceType || !matches(r, scope) || !matches(r, filter)) continue;
    evaluated += 1;
    for (const c of conditions) {
      const reason = conditionFailure(c, values(r, c.attribute));
      if (reason) {
        violations.push({ resource: r, reason });
        break;
      }
    }
  }
  violations.sort((a, b) => a.resource.name.localeCompare(b.resource.name) || a.resource.id.localeCompare(b.resource.id));
  return { evaluation: { evaluated, noncompliant: violations.length, score: score(evaluated, violations.length) }, violations };
}
