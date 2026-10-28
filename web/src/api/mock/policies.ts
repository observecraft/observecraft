// In-browser stand-in for Infrastructure → Resources → Policies (see api/policies/client.ts): the
// server's templates, two demo policies with a month of daily snapshots, all evaluated on the mock catalog.
import type { CatalogFacet, CatalogResource } from '../catalog/client';
import { evaluate, score, validateTarget } from '../policies/logic';
import type {
  PolicyCategory,
  PolicyCondition,
  PolicyTemplate,
  ResourcePolicy,
  ResourcePolicyDetail,
  ResourcePolicyInput,
  ResourcePolicyList,
  ResourcePolicyPreview,
  ResourcePolicyPreviewRequest,
  ResourcePolicySnapshot,
  ResourcePolicyTypeAttributes,
} from '../policies/types';
import { parseQuery } from '../../lib/searchQuery';
import { catalogResources, matches, values } from './catalog';
import { mockMe } from './identity';

const DAY = 86400e3;
const FILTER_FACETS = ['team', 'cloud', 'region', 'env', 'account', 'service'];
const FIELDS = ['name', 'cloud', 'account', 'region', 'cluster', 'namespace', 'env', 'service', 'team', 'status'];

const delay = <T>(value: T, ms = 80 + Math.random() * 120) => new Promise<T>((r) => setTimeout(() => r(value), ms));
const cond = (attribute: string, operator: PolicyCondition['operator'], vals: string[] = []): PolicyCondition => ({ attribute, operator, values: vals });

/** The server's built-in templates (resource_policy_rules.rs), same keys and checks. */
export const TEMPLATES: PolicyTemplate[] = [
  {
    key: 'kube_no_latest_tag',
    name: "Kubernetes workloads don't run the latest image tag",
    description: 'A floating tag changes what runs on every restart. Pin each container image to a release tag or a digest.',
    category: 'versioning',
    priority: 2,
    target: { resourceType: 'kube_deployment', scope: '', conditions: [cond('image_tags', 'not_one_of', ['latest'])] },
  },
  {
    key: 'kube_requests_limits',
    name: 'Kubernetes workloads set CPU and memory requests and limits',
    description: "Without requests the scheduler can't place pods well; without limits one pod can starve its neighbours.",
    category: 'reliability',
    priority: 2,
    target: {
      resourceType: 'kube_deployment',
      scope: '',
      conditions: [cond('cpu_requests', 'exists'), cond('cpu_limits', 'exists'), cond('memory_requests', 'exists'), cond('memory_limits', 'exists')],
    },
  },
  {
    key: 'kube_prod_replicas',
    name: 'Production deployments run at least two replicas',
    description: 'A single replica means downtime on every rollout or node failure.',
    category: 'reliability',
    priority: 1,
    target: { resourceType: 'kube_deployment', scope: 'env:prod', conditions: [cond('replicas', 'gte', ['2'])] },
  },
  {
    key: 'kube_unified_tags',
    name: 'Kubernetes deployments carry team, service and env tags',
    description: 'Unified service tags link a workload to its owners, its traces and its logs.',
    category: 'tagging',
    priority: 3,
    target: { resourceType: 'kube_deployment', scope: '', conditions: [cond('team', 'exists'), cond('service', 'exists'), cond('env', 'exists')] },
  },
  {
    key: 'host_team_tag',
    name: 'Hosts have an owning team',
    description: 'Every host should carry a team tag so alerts and costs reach the right people.',
    category: 'tagging',
    priority: 3,
    target: { resourceType: 'host', scope: '', conditions: [cond('team', 'exists')] },
  },
  {
    key: 'host_supported_ubuntu',
    name: 'Ubuntu hosts run a supported LTS release',
    description: 'Ubuntu 20.04 and older are out of standard support. Upgrade hosts to 22.04 or later.',
    category: 'security',
    priority: 2,
    target: { resourceType: 'host', scope: 'distro:ubuntu', conditions: [cond('os_release', 'version_gte', ['22.04'])] },
  },
  {
    key: 'kube_node_version',
    name: 'Kubernetes nodes run a supported kubelet version',
    description: "Nodes on old minor versions miss security fixes. Adjust the minimum to your provider's support window.",
    category: 'versioning',
    priority: 2,
    target: { resourceType: 'kube_node', scope: '', conditions: [cond('version', 'version_gte', ['1.31'])] },
  },
  {
    key: 'kube_service_no_nodeport',
    name: "Kubernetes services aren't exposed through node ports",
    description: 'NodePort services open a port on every node. Prefer a load balancer or an ingress.',
    category: 'security',
    priority: 2,
    target: { resourceType: 'kube_service', scope: '', conditions: [cond('service_type', 'not_equals', ['NodePort'])] },
  },
  {
    key: 'kube_staging_replicas',
    name: 'Staging deployments run at most two replicas',
    description: 'Non-production environments rarely need more capacity than two replicas give.',
    category: 'cost_optimization',
    priority: 3,
    target: { resourceType: 'kube_deployment', scope: 'env:staging', conditions: [cond('replicas', 'lte', ['2'])] },
  },
  {
    key: 'ecs_service_tasks',
    name: 'ECS services run at least two tasks',
    description: 'A service with one running task goes down with its task.',
    category: 'reliability',
    priority: 2,
    target: { resourceType: 'ecs_service', scope: '', conditions: [cond('tasks', 'gte', ['2'])] },
  },
];

interface Stored {
  id: string;
  name: string;
  description: string;
  priority: number;
  category: PolicyCategory;
  resourceType: ResourcePolicy['resourceType'];
  scope: string;
  conditions: PolicyCondition[];
  template: string | null;
  createdBy: string | null;
  createdAt: number;
  updatedAt: number;
  history: { day: string; evaluated: number; noncompliant: number }[];
}

let store: Stored[] | null = null;
let seq = 0;

const dayKey = (t: number) => new Date(t).toISOString().slice(0, 10);

function fromTemplate(key: string, id: string, ageDays: number, trend: (daysAgo: number, evaluated: number, now: number) => number): Stored {
  const t = TEMPLATES.find((x) => x.key === key)!;
  const now = Date.now();
  const current = evaluate(t.target, catalogResources()).evaluation;
  return {
    id,
    name: t.name,
    description: t.description,
    priority: t.priority,
    category: t.category,
    resourceType: t.target.resourceType,
    scope: t.target.scope ?? '',
    conditions: t.target.conditions,
    template: t.key,
    createdBy: 'Priya Shah',
    createdAt: now - ageDays * DAY,
    updatedAt: now - ageDays * DAY,
    history: Array.from({ length: ageDays + 1 }, (_, i) => {
      const daysAgo = ageDays - i;
      const evaluated = current.evaluated;
      return { day: dayKey(now - daysAgo * DAY), evaluated, noncompliant: daysAgo === 0 ? current.noncompliant : Math.max(0, Math.min(evaluated, trend(daysAgo, evaluated, current.noncompliant))) };
    }),
  };
}

function policies(): Stored[] {
  if (store) return store;
  // Requests and limits got fixed over the month; the latest tag crept in a week ago.
  store = [
    fromTemplate('kube_requests_limits', '6f0c2d1e-8a4b-4c1e-9b7a-000000000001', 30, (d, n, now) => now + Math.round((d / 30) * n * 0.6)),
    fromTemplate('kube_no_latest_tag', '6f0c2d1e-8a4b-4c1e-9b7a-000000000002', 21, (d, _n, now) => (d > 7 ? 0 : now)),
  ];
  return store;
}

function view(p: Stored, resources: CatalogResource[], query = ''): ResourcePolicy {
  const { history: _h, ...rest } = p;
  return { ...rest, evaluation: evaluate(p, resources, parseQuery(query)).evaluation };
}

function facets(all: CatalogResource[], query: string): CatalogFacet[] {
  const terms = parseQuery(query);
  return FILTER_FACETS.map((key) => {
    const others = terms.filter((t) => t.kind !== 'tag' || t.key !== key);
    const counts = new Map<string, number>();
    for (const r of all.filter((x) => matches(x, others))) for (const v of new Set(values(r, key))) counts.set(v, (counts.get(v) ?? 0) + 1);
    return { key, values: [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value)) };
  }).filter((f) => f.values.length);
}

export async function searchPolicies(query: string): Promise<ResourcePolicyList> {
  const all = catalogResources();
  const filter = parseQuery(query);
  return delay({
    policies: policies()
      .map((p) => view(p, all, query))
      .sort((a, b) => a.priority - b.priority || a.name.localeCompare(b.name)),
    templates: TEMPLATES.map((t) => ({ ...t, available: all.some((r) => r.resourceType === t.target.resourceType), preview: evaluate(t.target, all, filter).evaluation })),
    facets: facets(all, query),
  });
}

export async function policyAttributes(): Promise<ResourcePolicyTypeAttributes[]> {
  const by = new Map<string, { type: CatalogResource['resourceType']; n: number; keys: Map<string, { count: number; examples: Set<string> }> }>();
  for (const r of catalogResources()) {
    const e = by.get(r.resourceType) ?? { type: r.resourceType, n: 0, keys: new Map() };
    e.n += 1;
    const keys = new Set([...FIELDS, ...Object.keys(r.attributes), ...r.tags.map((t) => t.split(':')[0]).filter(Boolean)]);
    for (const k of keys) {
      const vals = values(r, k);
      if (!vals.length) continue;
      const a = e.keys.get(k) ?? { count: 0, examples: new Set<string>() };
      a.count += 1;
      for (const v of vals) if (a.examples.size < 5) a.examples.add(v);
      e.keys.set(k, a);
    }
    by.set(r.resourceType, e);
  }
  const field = (k: string) => (FIELDS.includes(k) ? FIELDS.indexOf(k) : Infinity);
  return delay(
    [...by.values()]
      .sort((a, b) => a.type.localeCompare(b.type))
      .map((e) => ({
        resourceType: e.type,
        resources: e.n,
        attributes: [...e.keys]
          .map(([key, a]) => ({ key, count: a.count, examples: [...a.examples].sort() }))
          .sort((a, b) => field(a.key) - field(b.key) || b.count - a.count || a.key.localeCompare(b.key))
          .slice(0, 150),
      })),
  );
}

export async function previewPolicy(req: ResourcePolicyPreviewRequest): Promise<ResourcePolicyPreview> {
  const problem = validateTarget(req);
  if (problem) throw new Error(problem);
  const { evaluation, violations } = evaluate(req, catalogResources());
  return delay({ evaluation, violations: violations.slice(0, 20) });
}

function find(id: string): Stored {
  const p = policies().find((x) => x.id === id);
  if (!p) throw new Error(`Policy ${id} not found`);
  return p;
}

export async function getPolicy(id: string, query: string): Promise<ResourcePolicyDetail> {
  const p = find(id);
  const all = catalogResources();
  const { evaluation, violations } = evaluate(p, all, parseQuery(query));
  const history: ResourcePolicySnapshot[] = p.history.map((h) => ({ ...h, score: score(h.evaluated, h.noncompliant) }));
  return delay({ policy: { ...view(p, all), evaluation }, history, violations: violations.slice(0, 500) });
}

function checked(input: ResourcePolicyInput, except?: string) {
  const name = input.name.trim();
  if (!name) throw new Error('The policy name is required');
  if (input.priority < 0 || input.priority > 4) throw new Error('The priority is P0 to P4');
  const problem = validateTarget(input);
  if (problem) throw new Error(problem);
  if (policies().some((p) => p.id !== except && p.name === name)) throw new Error(`A policy named '${name}' already exists`);
  return { ...input, name, description: (input.description ?? '').trim(), scope: (input.scope ?? '').trim() };
}

/** Today's snapshot, as the server stores it after each write. */
function snapshot(p: Stored) {
  const e = evaluate(p, catalogResources()).evaluation;
  const today = dayKey(Date.now());
  p.history = [...p.history.filter((h) => h.day !== today), { day: today, evaluated: e.evaluated, noncompliant: e.noncompliant }];
}

export async function createPolicy(input: ResourcePolicyInput): Promise<ResourcePolicy> {
  const c = checked(input);
  const now = Date.now();
  seq += 1;
  const p: Stored = {
    id: `6f0c2d1e-8a4b-4c1e-9b7a-${String(100 + seq).padStart(12, '0')}`,
    name: c.name,
    description: c.description,
    priority: c.priority,
    category: c.category,
    resourceType: c.resourceType,
    scope: c.scope,
    conditions: c.conditions,
    template: c.template && TEMPLATES.some((t) => t.key === c.template) ? c.template : null,
    createdBy: mockMe().name,
    createdAt: now,
    updatedAt: now,
    history: [],
  };
  snapshot(p);
  policies().push(p);
  return delay(view(p, catalogResources()));
}

export async function updatePolicy(id: string, input: ResourcePolicyInput): Promise<ResourcePolicy> {
  const p = find(id);
  const c = checked(input, id);
  Object.assign(p, {
    name: c.name,
    description: c.description,
    priority: c.priority,
    category: c.category,
    resourceType: c.resourceType,
    scope: c.scope,
    conditions: c.conditions,
    updatedAt: Date.now(),
  });
  snapshot(p);
  return delay(view(p, catalogResources()));
}

export async function deletePolicy(id: string): Promise<void> {
  find(id);
  store = policies().filter((p) => p.id !== id);
  return delay(undefined);
}
