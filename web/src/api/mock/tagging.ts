// In-browser tagging policies and provenance for mock mode: the e-commerce demo's cloud resources (Terraform in
// example-org/infra-terraform, some tagged by Yor, some not at all), its Kubernetes workloads deployed by Argo CD
// (example-org/k8s-apps), its images and its services. Evaluated with the same rules as the server (tagging/logic.ts).
import type {
  Provenance,
  ProvenanceList,
  ProvenanceSummary,
  RecommendedView,
  TagCoverage,
  TaggedResource,
  TaggingPolicy,
  TaggingPolicyInput,
  TaggingPolicyList,
  TagRule,
  TagTarget,
} from '../tagging/types';
import { coverage as evaluate, count, applies, tagValues } from '../tagging/logic';

const delay = <T>(v: T, ms = 120): Promise<T> => new Promise((r) => setTimeout(() => r(v), ms));
const now = Date.now();
const day = 86_400_000;

// ---------------------------------------------------------------- Argo CD (Kubernetes provenance)

interface ArgoApp {
  name: string;
  repo: string;
  path: string;
  ref: string;
  kind: Provenance['kind'];
}

/** Applications collected from the ops cluster (the Cluster Agent's custom resource collection). `payments` isn't. */
const ARGO_APPS: ArgoApp[] = [
  { name: 'storefront', repo: 'github.com/example-org/k8s-apps', path: 'apps/storefront/overlays/prod', ref: 'main', kind: 'kustomize' },
  { name: 'checkout', repo: 'github.com/example-org/k8s-apps', path: 'apps/checkout/overlays/prod', ref: 'main', kind: 'kustomize' },
  { name: 'catalog', repo: 'github.com/example-org/helm-charts', path: 'charts/catalog', ref: 'v3.2.1', kind: 'helm' },
];

/** How the demo's workloads are tracked by Argo CD in production (staging apps are `<app>-staging`, not collected). */
const TRACKING: Record<string, { app: string; label?: boolean }> = {
  'api-gateway': { app: 'storefront' },
  'storefront-api': { app: 'storefront' },
  'order-service': { app: 'checkout' },
  'catalog-service': { app: 'catalog' },
  'inventory-service': { app: 'catalog', label: true },
  billing: { app: 'payments' },
};

/** A demo Kubernetes object's Argo CD metadata and resolved source; null when Argo CD doesn't deploy it. */
export function kubeProvenance(
  kind: 'Deployment' | 'Service',
  name: string,
  namespace: string,
  cluster: string,
): { annotations: Record<string, string>; labels: Record<string, string>; provenance: Provenance } | null {
  const t = TRACKING[name];
  if (!t) return null;
  const staging = cluster.includes('staging');
  const app = staging ? `${t.app}-staging` : t.app;
  const tracking = `${app}:${kind === 'Deployment' ? 'apps' : ''}/${kind}:${namespace}/${name}`;
  const found = staging ? undefined : ARGO_APPS.find((a) => a.name === app);
  const provenance: Provenance = {
    repo: found?.repo ?? null,
    path: found?.path ?? null,
    ref: found?.ref ?? null,
    address: `${kind}/${namespace}/${name}`,
    application: app,
    kind: found?.kind ?? 'manifest',
    confidence: t.label && !found ? 'inferred' : 'exact',
    via: 'argocd',
  };
  return t.label ? { annotations: {}, labels: { 'app.kubernetes.io/instance': app }, provenance } : { annotations: { 'argocd.argoproj.io/tracking-id': tracking }, labels: {}, provenance };
}

// ---------------------------------------------------------------- resources

const tf = (path: string): Provenance => ({ repo: 'github.com/example-org/infra-terraform', path, ref: null, address: null, application: null, kind: 'terraform', confidence: 'exact', via: 'tag' });

function cloud(provider: string, account: string, id: string, name: string, resourceType: string, region: string, tags: string[], provenance: Provenance | null = null): TaggedResource {
  return {
    id,
    target: 'cloud',
    name,
    resourceType,
    provider,
    account,
    region,
    cluster: null,
    namespace: null,
    env: tagValues(tags, 'env')[0] ?? null,
    team: tagValues(tags, 'team')[0] ?? null,
    tags,
    provenance,
  };
}

const AWS = '123456789012';
const AZURE = '00000000-0000-0000-0000-00000000a001';
const GCP = 'example-shop-prod';

const CLOUD: TaggedResource[] = [
  cloud(
    'aws',
    AWS,
    'arn:aws:s3:::example-shop-assets',
    'example-shop-assets',
    'AwsS3Bucket',
    'eu-west-1',
    ['managed-by:terraform', 'tf_repo:github.com/example-org/infra-terraform', 'tf_path:envs/prod/storage', 'team:storefront', 'env:prod'],
    tf('envs/prod/storage'),
  ),
  cloud(
    'aws',
    AWS,
    'arn:aws:rds:eu-west-1:123456789012:db:orders-prod',
    'orders-prod',
    'AwsRdsDbInstance',
    'eu-west-1',
    [
      'managed-by:terraform',
      'git_org:example-org',
      'git_repo:infra-terraform',
      'git_file:modules/orders-db/main.tf',
      'git_commit:4f1c2a9e0b7d',
      'yor_name:orders',
      'yor_trace:7e3c1a52-9b0d-4c6e-8f21-2d5a4b3c1e00',
      'team:checkout',
      'env:prod',
    ],
    { repo: 'example-org/infra-terraform', path: 'modules/orders-db/main.tf', ref: '4f1c2a9e0b7d', address: 'orders', application: null, kind: 'terraform', confidence: 'exact', via: 'tag' },
  ),
  cloud(
    'aws',
    AWS,
    'arn:aws:sqs:eu-west-1:123456789012:order-events',
    'order-events',
    'AwsSqsQueue',
    'eu-west-1',
    ['managed-by:terraform', 'tf_repo:github.com/example-org/infra-terraform', 'tf_path:envs/prod/messaging', 'team:checkout', 'env:prod'],
    tf('envs/prod/messaging'),
  ),
  cloud('aws', AWS, 'arn:aws:s3:::example-shop-logs', 'example-shop-logs', 'AwsS3Bucket', 'eu-west-1', ['managed-by:terraform', 'team:platform', 'env:production']),
  cloud('aws', AWS, 'arn:aws:ec2:eu-west-1:123456789012:instance/i-0a1b2c3d4e5f60007', 'i-0a1b2c3d4e5f60007', 'AwsEc2Instance', 'eu-west-1', ['Name:bastion']),
  cloud('aws', AWS, 'arn:aws:lambda:eu-west-1:123456789012:function:image-resizer', 'image-resizer', 'AwsLambdaFunction', 'eu-west-1', ['team:catalog', 'env:prod', 'managed-by:serverless']),
  cloud('aws', AWS, 'arn:aws:iam::123456789012:role/ci-deployer', 'ci-deployer', 'AwsIamRole', 'global', []),
  cloud(
    'azure',
    AZURE,
    `/subscriptions/${AZURE}/resourceGroups/shop-prod/providers/Microsoft.Storage/storageAccounts/shopexports`,
    'shopexports',
    'Microsoft.Storage/storageAccounts',
    'westeurope',
    ['Managed-By:terraform', 'TF-Repo:github.com/example-org/infra-terraform', 'TF-Path:envs/azure-prod/exports', 'team:billing', 'env:prod'],
    tf('envs/azure-prod/exports'),
  ),
  cloud('azure', AZURE, `/subscriptions/${AZURE}/resourceGroups/shop-prod/providers/Microsoft.KeyVault/vaults/shop-secrets`, 'shop-secrets', 'Microsoft.KeyVault/vaults', 'westeurope', ['env:prod']),
  cloud('gcp', GCP, '//storage.googleapis.com/projects/_/buckets/example-shop-backups', 'example-shop-backups', 'storage.googleapis.com/Bucket', 'europe-west1', ['managed-by:terraform', 'env:prod']),
];

const WORKLOADS: [string, string, string | null][] = [
  ['api-gateway', 'checkout', 'checkout'],
  ['storefront-api', 'checkout', 'checkout'],
  ['order-service', 'checkout', 'checkout'],
  ['catalog-service', 'catalog', 'catalog'],
  ['inventory-service', 'catalog', 'catalog'],
  ['billing', 'payments', 'payments'],
  ['workflow-engine', 'payments', 'payments'],
  ['notification-worker', 'checkout', null],
];

function kube(): TaggedResource[] {
  const out: TaggedResource[] = [];
  for (const cluster of ['shop-prod-eu', 'shop-staging']) {
    const env = cluster.includes('prod') ? 'prod' : 'staging';
    for (const [name, ns, team] of WORKLOADS) {
      for (const kind of ['Deployment', 'Service'] as const) {
        const meta = kubeProvenance(kind, name, ns, cluster);
        const tags = [
          `kube_cluster_name:${cluster}`,
          `kube_namespace:${ns}`,
          `service:${name}`,
          `env:${env}`,
          ...(team ? [`team:${team}`] : []),
          ...Object.entries(meta?.labels ?? {}).map(([k, v]) => `${k}:${v}`),
        ];
        tags.push(...Object.entries(meta?.annotations ?? {}).map(([k, v]) => `${k}:${v}`));
        out.push({
          id: `${cluster}-${kind === 'Deployment' ? 'dep' : 'svc'}-${name}`,
          target: 'kubernetes',
          name,
          resourceType: kind === 'Deployment' ? 'kube_deployment' : 'kube_service',
          provider: null,
          account: null,
          region: 'eu-west-1',
          cluster,
          namespace: ns,
          env,
          team,
          tags,
          provenance: meta?.provenance ?? null,
        });
      }
    }
  }
  return out;
}

const oci = (repo: string, ref: string): Provenance => ({ repo, path: null, ref, address: null, application: null, kind: 'dockerfile', confidence: 'exact', via: 'oci' });

function image(name: string, tags: string[], provenance: Provenance | null): TaggedResource {
  return {
    id: `sha256:${name.length.toString(16).padStart(4, '0')}${name.replace(/[^a-z]/g, '').slice(0, 12)}`,
    target: 'image',
    name,
    resourceType: 'container_image',
    provider: null,
    account: null,
    region: null,
    cluster: 'shop-prod-eu',
    namespace: null,
    env: 'prod',
    team: tagValues(tags, 'team')[0] ?? null,
    tags,
    provenance,
  };
}

const IMAGES: TaggedResource[] = [
  image(
    'registry.example.com/storefront-api:2.4.0',
    ['image_name:registry.example.com/storefront-api', 'org.opencontainers.image.source:https://github.com/example-org/storefront-api', 'org.opencontainers.image.revision:9c1e7b2', 'team:checkout'],
    oci('github.com/example-org/storefront-api', '9c1e7b2'),
  ),
  image(
    'registry.example.com/order-service:2.4.0',
    ['image_name:registry.example.com/order-service', 'git.repository_url:github.com/example-org/order-service', 'git.commit.sha:4d2a0f1', 'team:checkout'],
    oci('github.com/example-org/order-service', '4d2a0f1'),
  ),
  image(
    'registry.example.com/billing:3.2.1',
    ['image_name:registry.example.com/billing', 'org.opencontainers.image.source:https://github.com/example-org/billing', 'team:payments'],
    oci('github.com/example-org/billing', ''),
  ),
  image('registry.example.com/notification-worker:latest', ['image_name:registry.example.com/notification-worker'], null),
  image('registry.example.com/catalog-service:2.4.0', ['image_name:registry.example.com/catalog-service', 'team:catalog'], null),
];
IMAGES[2].provenance!.ref = null;

const git = (repo: string, sha: string): Provenance => ({ repo, path: null, ref: sha, address: null, application: null, kind: 'code', confidence: 'exact', via: 'git_metadata' });

function service(name: string, team: string | null, version: string | null, repo: string | null, sha: string | null): TaggedResource {
  const tags = [
    `service:${name}`,
    'env:prod',
    ...(version ? [`version:${version}`] : []),
    ...(team ? [`team:${team}`] : []),
    ...(repo ? [`git.repository_url:https://${repo}`] : []),
    ...(sha ? [`git.commit.sha:${sha}`] : []),
  ];
  return {
    id: `service:${name}`,
    target: 'service',
    name,
    resourceType: 'service',
    provider: null,
    account: null,
    region: null,
    cluster: null,
    namespace: null,
    env: 'prod',
    team,
    tags,
    provenance: repo ? git(repo, sha ?? '') : null,
  };
}

const SERVICES: TaggedResource[] = [
  service('api-gateway', 'checkout', '2.4.0', 'github.com/example-org/api-gateway', 'a81f3c0'),
  service('storefront-api', 'checkout', '2.4.0', 'github.com/example-org/storefront-api', '9c1e7b2'),
  service('order-service', 'checkout', '2.4.0', 'github.com/example-org/order-service', '4d2a0f1'),
  service('catalog-service', 'catalog', '2.4.0', null, null),
  service('inventory-service', 'catalog', null, null, null),
  service('billing', 'payments', '3.2.1', 'github.com/example-org/billing', null),
];

const ALL = (): TaggedResource[] => [...CLOUD, ...kube(), ...IMAGES, ...SERVICES];

// ---------------------------------------------------------------- policies

const req = (key: string, allowedValues: string[] = []) => ({ key, allowedValues });
const scope = { provider: '', account: '', cluster: '', resourceType: '', names: '' };

/** The server's recommended set (tagging_rules.rs `recommended_set`). */
const RECOMMENDED: { key: string; name: string; description: string; rule: TagRule }[] = [
  {
    key: 'services_unified_tagging',
    name: 'Services carry unified service tags and git metadata',
    description: 'service, env and version link a service’s traces, logs and metrics; the repository and commit (DD_GIT_REPOSITORY_URL, DD_GIT_COMMIT_SHA) tell where to fix it.',
    rule: { target: 'service', scope, required: [req('service'), req('env'), req('version'), req('git.repository_url'), req('git.commit.sha')] },
  },
  {
    key: 'cloud_terraform_source',
    name: 'Cloud resources name their Terraform source',
    description: 'managed-by, and the repository and module path (tf_repo, tf_path, or Yor’s git_repo, git_file) on every resource Terraform creates.',
    rule: { target: 'cloud', scope, required: [req('managed-by'), req('tf_repo|git_repo'), req('tf_path|git_file')] },
  },
  {
    key: 'cloud_ownership',
    name: 'Cloud resources have an owning team and an environment',
    description: 'Alerts, findings and costs reach the right people when every resource carries team and env.',
    rule: { target: 'cloud', scope, required: [req('team'), req('env', ['prod', 'staging', 'dev'])] },
  },
  {
    key: 'kubernetes_argocd_tracking',
    name: 'Kubernetes objects are tracked by Argo CD',
    description: 'Argo CD’s tracking annotation (or label) names the Application whose repository, path and revision define the object.',
    rule: { target: 'kubernetes', scope, required: [req('argocd.argoproj.io/tracking-id|app.kubernetes.io/instance')] },
  },
  {
    key: 'images_oci_source',
    name: 'Images carry their OCI source and revision',
    description: 'The org.opencontainers.image.source and .revision labels, set at build time, name the repository and commit an image was built from.',
    rule: { target: 'image', scope, required: [req('org.opencontainers.image.source|git.repository_url'), req('org.opencontainers.image.revision|git.commit.sha')] },
  },
];

let seq = 0;
const policies: TaggingPolicy[] = RECOMMENDED.map((r) => ({
  id: `tp-${++seq}`,
  name: r.name,
  description: r.description,
  target: r.rule.target,
  scope: { ...r.rule.scope },
  required: r.rule.required.map((t) => ({ ...t })),
  enabled: true,
  recommended: r.key,
  createdBy: null,
  createdAt: now - 30 * day,
  updatedAt: now - 30 * day,
  coverage: count(0, 0),
}));

const asRule = (p: TaggingPolicy): TagRule => ({ target: p.target, scope: p.scope, required: p.required });

function withCoverage(p: TaggingPolicy): TaggingPolicy {
  return { ...p, coverage: evaluate([{ id: p.id, name: p.name, rule: asRule(p) }], ALL(), 0).total };
}

function validate(input: TaggingPolicyInput, id?: string): TaggingPolicyInput {
  const name = input.name.trim();
  if (!name) throw new Error('The policy name is required');
  if (policies.some((p) => p.id !== id && p.name.toLowerCase() === name.toLowerCase())) throw new Error(`A tagging policy named '${name}' already exists`);
  const required = (input.required ?? []).map((t) => ({
    key: t.key
      .split('|')
      .map((k) => k.trim())
      .filter(Boolean)
      .join('|'),
    allowedValues: (t.allowedValues ?? []).map((v) => v.trim()).filter(Boolean),
  }));
  if (!required.length) throw new Error('A policy needs at least one required tag');
  const bad = required.find((t) => !t.key || /[:\s]/.test(t.key));
  if (bad) throw new Error(`'${bad.key}' is not a valid tag key`);
  return { ...input, name, required, scope: { ...scope, ...input.scope } };
}

export async function listPolicies(): Promise<TaggingPolicyList> {
  const recommended: RecommendedView[] = RECOMMENDED.map((r) => ({ ...r, installed: policies.some((p) => p.recommended === r.key) }));
  return delay({ policies: policies.map(withCoverage), recommended });
}

export async function getPolicy(id: string): Promise<TaggingPolicy> {
  const p = policies.find((x) => x.id === id);
  if (!p) throw new Error(`Tagging policy ${id} not found`);
  return delay(withCoverage(p));
}

export async function createPolicy(input: TaggingPolicyInput): Promise<TaggingPolicy> {
  const v = validate(input);
  const p: TaggingPolicy = {
    id: `tp-${++seq}`,
    name: v.name,
    description: v.description ?? '',
    target: v.target,
    scope: v.scope!,
    required: v.required,
    enabled: v.enabled ?? true,
    recommended: RECOMMENDED.some((r) => r.key === v.recommended) ? v.recommended! : null,
    createdBy: 'Demo User',
    createdAt: Date.now(),
    updatedAt: Date.now(),
    coverage: count(0, 0),
  };
  policies.push(p);
  return delay(withCoverage(p));
}

export async function updatePolicy(id: string, input: TaggingPolicyInput): Promise<TaggingPolicy> {
  const p = policies.find((x) => x.id === id);
  if (!p) throw new Error(`Tagging policy ${id} not found`);
  const v = validate(input, id);
  Object.assign(p, { name: v.name, description: v.description ?? '', target: v.target, scope: v.scope, required: v.required, enabled: v.enabled ?? true, updatedAt: Date.now() });
  return delay(withCoverage(p));
}

export async function deletePolicy(id: string): Promise<void> {
  const i = policies.findIndex((x) => x.id === id);
  if (i < 0) throw new Error(`Tagging policy ${id} not found`);
  policies.splice(i, 1);
  return delay(undefined, 60);
}

function summarize(rows: TaggedResource[]): ProvenanceSummary {
  const s: ProvenanceSummary = { total: 0, resolved: 0, byTarget: {}, byVia: {} };
  for (const r of rows) {
    s.total++;
    const t = (s.byTarget[r.target] ??= [0, 0]);
    t[0]++;
    if (r.provenance) {
      s.resolved++;
      t[1]++;
      s.byVia[r.provenance.via] = (s.byVia[r.provenance.via] ?? 0) + 1;
    }
  }
  return s;
}

export async function coverage(policyId?: string): Promise<TagCoverage> {
  const chosen = policyId ? policies.filter((p) => p.id === policyId).map((p) => ({ ...p, enabled: true })) : policies.filter((p) => p.enabled);
  if (policyId && !chosen.length) throw new Error(`Tagging policy ${policyId} not found`);
  const evaluated = chosen.map((p) => ({ id: p.id, name: p.name, rule: asRule(p) }));
  const resources = ALL();
  const c = evaluate(evaluated, resources, 500);
  const inScope = resources.filter((r) => evaluated.some((p) => applies(p.rule, r)));
  // Coverage improved over the last weeks as teams added default_tags and OCI labels.
  const score = c.total.score ?? 0;
  const history = Array.from({ length: 45 }, (_, i) => {
    const d = new Date(now - (44 - i) * day).toISOString().slice(0, 10);
    const e = c.total.evaluated;
    const compliant = Math.max(0, Math.min(e, Math.round((e * Math.max(10, score - (44 - i) * 0.6 + Math.sin(i / 3) * 1.5)) / 100)));
    return { day: d, ...count(e, i === 44 ? c.total.compliant : compliant) };
  });
  return delay({ ...c, history, provenance: summarize(inScope), argocdApplications: ARGO_APPS.length });
}

export async function provenance(target?: TagTarget, missing = false): Promise<ProvenanceList> {
  const rows = ALL().filter((r) => !target || r.target === target);
  return delay({ summary: summarize(rows), resources: rows.filter((r) => !missing || !r.provenance) });
}

/** Cloud resources, Kubernetes objects and images failing the enabled policies (Cloud Security's `ocs-tag-001`). */
export function misconfigGaps(): { evaluated: number; gaps: TagCoverage['noncompliant'] } {
  const chosen = policies.filter((p) => p.enabled && p.target !== 'service').map((p) => ({ id: p.id, name: p.name, rule: asRule(p) }));
  const c = evaluate(chosen, ALL(), Number.MAX_SAFE_INTEGER);
  return { evaluated: c.total.evaluated, gaps: c.noncompliant };
}
