// Labels, grouping and sorting of resource policies (Infrastructure → Resources → Policies).
import type { PolicyCategory, PolicyCondition, ResourcePolicy } from '../../api/policies/types';
import { describeCondition } from '../../api/policies/logic';
import { TYPE_LABELS } from './catalogModel';

export const CATEGORY_NAMES: Record<PolicyCategory, string> = {
  cost_optimization: 'Cost Optimization',
  reliability: 'Reliability',
  security: 'Security',
  versioning: 'Versioning',
  tagging: 'Tagging & Ownership',
};

export const PRIORITIES = [0, 1, 2, 3, 4];

export type PolicySortField = 'name' | 'priority' | 'category' | 'resource_type' | 'attribute' | 'noncompliant' | 'score' | 'created_by';
export type PolicyGroupBy = 'none' | 'category' | 'priority' | 'resource_type' | 'created_by';

/** The attribute column: the first condition's attribute, "+N" for the others. */
export function targetAttribute(conditions: PolicyCondition[]): string {
  if (!conditions.length) return '—';
  const rest = new Set(
    conditions
      .slice(1)
      .map((c) => c.attribute)
      .filter((a) => a !== conditions[0].attribute),
  );
  return rest.size ? `${conditions[0].attribute} +${rest.size}` : conditions[0].attribute;
}

/** All conditions as one sentence. */
export function ruleSummary(conditions: PolicyCondition[]): string {
  return conditions.map(describeCondition).join(' and ');
}

function sortValue(p: ResourcePolicy, field: PolicySortField): string | number {
  switch (field) {
    case 'priority':
      return p.priority;
    case 'category':
      return CATEGORY_NAMES[p.category];
    case 'resource_type':
      return TYPE_LABELS[p.resourceType] ?? p.resourceType;
    case 'attribute':
      return targetAttribute(p.conditions);
    case 'noncompliant':
      return p.evaluation.noncompliant;
    case 'score':
      // Policies without resources sort after every score.
      return p.evaluation.score ?? 101;
    case 'created_by':
      return p.createdBy ?? '';
    default:
      return p.name.toLowerCase();
  }
}

export function sortPolicies(list: ResourcePolicy[], field: PolicySortField, desc: boolean): ResourcePolicy[] {
  return [...list].sort((a, b) => {
    const x = sortValue(a, field);
    const y = sortValue(b, field);
    const o = x < y ? -1 : x > y ? 1 : 0;
    return (desc ? -o : o) || a.name.localeCompare(b.name);
  });
}

/** Policies in groups (in the order groups first appear after sorting); one unnamed group when ungrouped. */
export function groupPolicies(list: ResourcePolicy[], by: PolicyGroupBy): { key: string; label: string; policies: ResourcePolicy[] }[] {
  if (by === 'none') return [{ key: '', label: '', policies: list }];
  const label = (p: ResourcePolicy) =>
    by === 'category' ? CATEGORY_NAMES[p.category] : by === 'priority' ? `P${p.priority}` : by === 'resource_type' ? (TYPE_LABELS[p.resourceType] ?? p.resourceType) : (p.createdBy ?? 'Unknown');
  const groups = new Map<string, ResourcePolicy[]>();
  for (const p of list) groups.set(label(p), [...(groups.get(label(p)) ?? []), p]);
  return [...groups].sort((a, b) => a[0].localeCompare(b[0])).map(([key, policies]) => ({ key, label: key, policies }));
}

/** "Showing 1–2 of 2 policies". */
export function showing(from: number, to: number, total: number): string {
  return `Showing ${total ? from : 0}–${to} of ${total} ${total === 1 ? 'policy' : 'policies'}`;
}

/** Score tone for bars and numbers. */
export function scoreTone(score: number | null | undefined): 'ok' | 'warn' | 'bad' | 'none' {
  if (score == null) return 'none';
  return score >= 90 ? 'ok' : score >= 60 ? 'warn' : 'bad';
}
