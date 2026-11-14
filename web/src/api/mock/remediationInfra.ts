// In-browser Fix in infrastructure for mock mode (routes/remediation_infra.rs): a cloud finding's resource resolved to
// its source through the demo's tags, Argo CD Applications and OCI labels (mock/tagging.ts), its level, the tags it
// lacks with the snippet that adds them, and the changes the demo's fixes propose. The demo's sources:
//   - example-shop-assets (S3, posture check s3_bucket_level_public_access_block) → example-org/infra-terraform by tags,
//     fixed without an LLM (its public access block);
//   - catalog-service (StatefulSet) → Argo CD Application catalog → example-org/gitops Helm values;
//   - storefront-api (Deployment) → Application storefront → example-org/k8s-apps kustomize; order-service the same,
//     inferred (its source needs a confirmation);
//   - registry.example.com/storefront-api images → their Dockerfile (OCI labels);
//   - example-shop-logs, catalog-service's image, node-debugger: no source, the tags they lack.
import type { CodeChange } from '../assist/types';
import type { MissingTag, ProvenanceSnippet, Remediation } from '../remediation/client';
import { tagValues } from '../tagging/logic';
import type { Provenance, TaggedResource, TagTarget } from '../tagging/types';
import { allVulns, misconfigurations } from './cloudSecurity';
import { kubeProvenance, taggedResources } from './tagging';

export type Subject = 'cloud' | 'kubernetes' | 'image' | 'host';

export interface InfraFinding {
  kind: 'cloud_misconfiguration' | 'cloud_vulnerability';
  ruleId: string;
  title: string;
  resourceId: string;
  resourceName: string;
  resourceType: string;
  namespace: string | null;
  ecosystem?: string;
  pkg?: string;
  version?: string;
  fixed?: string | null;
}

export async function infraFinding(kind: string, id: string): Promise<InfraFinding | null> {
  if (kind === 'cloud_misconfiguration') {
    const f = (await misconfigurations('all')).findings.find((x) => x.key === id);
    return f ? { kind, ruleId: f.ruleId, title: f.ruleName, resourceId: f.resourceId, resourceName: f.resourceName, resourceType: f.resourceType, namespace: f.namespace ?? null } : null;
  }
  if (kind === 'cloud_vulnerability') {
    const v = allVulns().find((x) => x.key === id);
    return v
      ? {
          kind,
          ruleId: v.advisoryId,
          title: v.title,
          resourceId: v.resourceId,
          resourceName: v.resourceName,
          resourceType: v.resourceKind,
          namespace: null,
          ecosystem: v.ecosystem,
          pkg: v.package,
          version: v.version,
          fixed: v.fixed,
        }
      : null;
  }
  return null;
}

export const subjectOf = (resourceType: string): Subject =>
  resourceType === 'host' ? 'host' : resourceType === 'container_image' ? 'image' : resourceType.startsWith('kube_') ? 'kubernetes' : 'cloud';

const KUBE_KIND: Record<string, string> = {
  kube_deployment: 'Deployment',
  kube_statefulset: 'StatefulSet',
  kube_daemonset: 'DaemonSet',
  kube_cronjob: 'CronJob',
  kube_service: 'Service',
  kube_ingress: 'Ingress',
};
const imageBase = (name: string) => (name.split('/').pop() ?? name).split(/[:@]/)[0];

/** The finding's resource (as Tag Coverage lists it) and its source. */
export function resolve(f: InfraFinding): { subject: Subject; resource: TaggedResource | null; provenance: Provenance | null } {
  const subject = subjectOf(f.resourceType);
  const all = taggedResources();
  if (subject === 'cloud') {
    const resource = all.find((r) => r.id === f.resourceId || (r.target === 'cloud' && r.name === f.resourceName)) ?? null;
    return { subject, resource, provenance: resource?.provenance ?? null };
  }
  if (subject === 'image') {
    const resource = all.find((r) => r.target === 'image' && imageBase(r.name) === imageBase(f.resourceName)) ?? null;
    return { subject, resource, provenance: resource?.provenance ?? null };
  }
  if (subject === 'kubernetes') {
    const kind = KUBE_KIND[f.resourceType] ?? 'Deployment';
    const ns = f.namespace ?? 'default';
    const meta = kubeProvenance(kind as 'Deployment', f.resourceName, ns, 'shop-prod-eu');
    const tags = [`kube_cluster_name:shop-prod-eu`, `kube_namespace:${ns}`, ...Object.entries({ ...meta?.labels, ...meta?.annotations }).map(([k, v]) => `${k}:${v}`)];
    const resource: TaggedResource = {
      id: f.resourceId,
      target: 'kubernetes',
      name: f.resourceName,
      resourceType: f.resourceType,
      provider: null,
      account: null,
      region: null,
      cluster: 'shop-prod-eu',
      namespace: ns,
      env: 'prod',
      team: null,
      tags,
      provenance: meta?.provenance ?? null,
    };
    // The demo's inferred source: order-service matched by its instance label, the Application found by name.
    if (f.resourceName === 'order-service' && resource.provenance) resource.provenance = { ...resource.provenance, confidence: 'inferred' };
    return { subject, resource, provenance: resource.provenance ?? null };
  }
  return { subject, resource: null, provenance: null };
}

/** The connection reaching a repository: GitHub's can write, GitLab's is read-only, other hosts have none. */
export function connectionError(repo: string): string | null {
  const host = repo.split('/')[0];
  if (host === 'github.com' || host === 'gitlab.com') return null;
  return `No GitHub or GitLab connection reaches ${host}: add one in Developer Portal → Settings. Read access lets Assist propose a fix; opening a merge request also needs write access.`;
}

type InfraLevel = Pick<Remediation, 'level' | 'reason' | 'source' | 'confirm' | 'connection'>;

/** As remediation_infra.rs `infra_level`. */
export function infraLevel(subject: Subject, p: Provenance | null): InfraLevel {
  const plain = (level: Remediation['level'], reason: string, source: Provenance | null = null): InfraLevel => ({ level, reason, source, confirm: false, connection: null });
  if (subject === 'host') return plain('explain', 'A host’s packages are upgraded on the host, or in the machine image it boots from: no repository is known to define them.');
  if (!p)
    return plain(
      'needs_provenance',
      'No tag or label on the resource names the repository and file that define it, so Assist can explain the finding but not fix it at its source. Add the tags below where the resource is defined; Tagging Policies list them.',
    );
  if (!p.repo)
    return plain(
      'needs_provenance',
      `Argo CD Application \`${p.application}\` deploys this object, but the Application itself isn’t collected, so its repository and path are unknown. Collect Argo CD Applications with the Cluster Agent (orchestrator custom resources: argoproj.io/v1alpha1/applications).`,
      p,
    );
  if (!p.path && p.kind !== 'dockerfile')
    return plain('needs_provenance', `The resource names its repository (${p.repo}) but not its module path: add tf_path (or Yor’s git_file) so Assist knows which Terraform files to open.`, p);
  const conn = connectionError(p.repo);
  if (conn)
    return {
      level: 'explain',
      reason: `The resource comes from ${p.repo}, which none of the organization’s source code connections reaches, so Assist can explain the finding but not open its source.`,
      source: p,
      confirm: false,
      connection: conn,
    };
  const at = p.path ? `${p.repo}/${p.path}` : p.repo;
  const app = p.application ? ` of Argo CD Application ${p.application}` : '';
  let reason =
    p.kind === 'terraform'
      ? `Assist opens the Terraform resource${p.address ? ` ${p.address}` : ''} in ${at} and proposes the change.`
      : p.kind === 'helm'
        ? `Assist opens the Helm values${app} in ${at} and proposes the change.`
        : p.kind === 'dockerfile'
          ? `Assist opens the Dockerfile in ${at} and proposes a newer base image that fixes the image’s OS packages; the image must then be rebuilt and redeployed.`
          : `Assist opens the manifests${app} in ${at} and proposes the change.`;
  const inferred = p.confidence === 'inferred';
  if (inferred) reason += ' The source was inferred, not named by the resource: confirm it before Assist edits it.';
  return { level: 'fix_in_infra', reason, source: p, confirm: inferred, connection: null };
}

const PROVENANCE_POLICIES: Partial<Record<TagTarget, [string, string[]]>> = {
  cloud: ['Cloud resources name their Terraform source', ['tf_repo|git_repo', 'tf_path|git_file']],
  kubernetes: ['Kubernetes objects are tracked by Argo CD', ['argocd.argoproj.io/tracking-id|app.kubernetes.io/instance']],
  image: ['Images carry their OCI source and revision', ['org.opencontainers.image.source|git.repository_url', 'org.opencontainers.image.revision|git.commit.sha']],
};

/** The provenance tags the resource lacks (the recommended policies'). */
export function missingTags(target: TagTarget, resource: TaggedResource | null): MissingTag[] {
  const [policy, keys] = PROVENANCE_POLICIES[target] ?? ['', []];
  return keys.filter((k) => !k.split('|').some((a) => tagValues(resource?.tags ?? [], a).length)).map((key) => ({ policy, key, value: null }));
}

const TF_TAGS = 'managed-by = "terraform"\n      tf_repo    = "github.com/<owner>/<repository>"\n      tf_path    = "<this root module’s directory>"';

/** As remediation_infra.rs `provenance_snippet`. */
export function snippet(target: TagTarget, resource: TaggedResource | null, missing: MissingTag[]): ProvenanceSnippet {
  const head = missing.length ? `# Missing: ${missing.map((m) => m.key).join(', ')}\n` : '';
  if (target === 'kubernetes')
    return {
      title: 'Argo CD: annotation tracking, and the Applications collected by the Cluster Agent',
      language: 'yaml',
      code: `${head}# argocd-cm\ndata:\n  application.resourceTrackingMethod: annotation\n\n# Datadog Helm chart values\ndatadog:\n  orchestratorExplorer:\n    enabled: true\n    customResources:\n      - argoproj.io/v1alpha1/applications\n`,
    };
  if (target === 'image')
    return {
      title: 'Images: the OCI source and revision labels, set at build time',
      language: 'shell',
      code: `${head}docker build \\\n  --label org.opencontainers.image.source="https://github.com/<owner>/<repository>" \\\n  --label org.opencontainers.image.revision="$(git rev-parse HEAD)" \\\n  -t ${resource?.name ?? 'registry.example.com/<image>:<tag>'} .\n`,
    };
  if (resource?.provider === 'azure' || resource?.provider === 'gcp')
    return {
      title: 'Terraform on Azure: a merged locals map (the azurerm provider has no default tags)',
      language: 'hcl',
      code: `${head}locals {\n  default_tags = {\n    ${TF_TAGS.replace(/\n {6}/g, '\n    ')}\n  }\n}\n\n# In every resource of the module, e.g. ${resource.name}:\n  tags = merge(local.default_tags, { purpose = "..." })\n`,
    };
  return {
    title: 'Terraform on AWS: the provider’s default_tags, once per root module',
    language: 'hcl',
    code: `${head}provider "aws" {\n  default_tags {\n    tags = {\n      ${TF_TAGS}\n    }\n  }\n}\n`,
  };
}

// ---------------------------------------------------------------- the demo's changes

export const S3_PUBLIC_ACCESS_CHECKS = ['s3_bucket_level_public_access_block', 's3_bucket_public_access'];

const ASSETS_TF = `resource "aws_s3_bucket" "assets" {
  bucket = "example-shop-assets"
}

resource "aws_s3_bucket_public_access_block" "assets" {
  bucket = aws_s3_bucket.assets.id

  block_public_acls       = false
  block_public_policy     = true
}
`;

/** The S3 bucket's public access block, as the deterministic editor writes it. */
export function s3Change(): { change: CodeChange; located: { file: string; startLine: number; endLine: number; address: string; by: string } } {
  const after = ASSETS_TF.replace('block_public_acls       = false', 'block_public_acls       = true').replace(
    '  block_public_policy     = true\n}',
    '  block_public_policy     = true\n  ignore_public_acls = true\n  restrict_public_buckets = true\n}',
  );
  return {
    change: {
      title: 'Block public access to the example-shop-assets bucket',
      description:
        'Sets `block_public_acls`, `block_public_policy`, `ignore_public_acls` and `restrict_public_buckets` to true for `example-shop-assets`, defined at `aws_s3_bucket.assets` in `envs/prod/storage/s3.tf`.',
      files: [{ path: 'envs/prod/storage/s3.tf', before: ASSETS_TF, after }],
    },
    located: { file: 'envs/prod/storage/s3.tf', startLine: 1, endLine: 3, address: 'aws_s3_bucket.assets', by: 'name' },
  };
}

/** What each Kubernetes rule sets in a container's security context or spec. */
const K8S_FIX: Record<string, [string, string]> = {
  'ocs-k8s-005': ['runAsNonRoot', 'true'],
  'ocs-k8s-006': ['allowPrivilegeEscalation', 'false'],
  'ocs-k8s-009': ['readOnlyRootFilesystem', 'true'],
  'ocs-k8s-001': ['privileged', 'false'],
};

/** The change an Assist run proposes for the demo's Kubernetes and image findings. */
export function assistChange(f: InfraFinding, p: Provenance): CodeChange {
  if (p.kind === 'dockerfile') {
    const before = `FROM maven:3.9-eclipse-temurin-21 AS build\nWORKDIR /src\nCOPY . .\nRUN mvn -q package -DskipTests\n\nFROM debian:12.4-slim\nRUN apt-get update && apt-get install -y --no-install-recommends openjdk-17-jre-headless && rm -rf /var/lib/apt/lists/*\nCOPY --from=build /src/target/app.jar /app.jar\nUSER 10001\nENTRYPOINT ["java", "-jar", "/app.jar"]\n`;
    return {
      title: `Bump the base image of ${imageBase(f.resourceName)} to debian:12.8-slim`,
      description: `\`debian:12.8-slim\` ships ${f.pkg ?? 'the package'} ${f.fixed ?? 'with the fix'} (the image has ${f.version ?? 'an affected version'}). The image must be rebuilt, rescanned and redeployed.`,
      files: [{ path: 'Dockerfile', before, after: before.replace('FROM debian:12.4-slim', 'FROM debian:12.8-slim') }],
    };
  }
  const [key, value] = K8S_FIX[f.ruleId] ?? ['readOnlyRootFilesystem', 'true'];
  if (p.kind === 'helm') {
    const path = `${p.path}/values-prod.yaml`;
    const before = `replicaCount: 3\n\nimage:\n  repository: registry.example.com/catalog-service\n  tag: "2.4.0"\n\nsearchIndexer:\n  image:\n    repository: registry.example.com/search-indexer\n    tag: latest\n\nresources:\n  limits:\n    cpu: "1"\n    memory: 1Gi\n`;
    const after =
      f.ruleId === 'ocs-k8s-007' ? before.replace('    tag: latest', '    tag: "1.9.3"') : before.replace('replicaCount: 3\n', `replicaCount: 3\n\ncontainerSecurityContext:\n  ${key}: ${value}\n`);
    return {
      title: f.ruleId === 'ocs-k8s-007' ? 'Pin the search-indexer image' : `Set ${key}: ${value} for ${f.resourceName}`,
      description:
        f.ruleId === 'ocs-k8s-007'
          ? 'Pins `searchIndexer.image.tag` to the version running now instead of `latest` (the chart’s statefulset template reads it).'
          : `Sets \`containerSecurityContext.${key}\` in the prod values; the chart’s statefulset template renders it into every container.`,
      files: [{ path, before, after }],
    };
  }
  const path = `${p.path}/security-context.yaml`;
  const after = `apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: ${f.resourceName}\nspec:\n  template:\n    spec:\n      containers:\n        - name: ${f.resourceName}\n          securityContext:\n            ${key}: ${value}\n`;
  const kustomization = `apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\nresources:\n  - ../../base\npatches:\n  - path: replicas.yaml\n`;
  return {
    title: `Set ${key}: ${value} for ${f.resourceName}`,
    description: `Adds a strategic merge patch to the prod overlay that sets \`securityContext.${key}: ${value}\` on the ${f.resourceName} container.`,
    files: [
      { path: `${p.path}/kustomization.yaml`, before: kustomization, after: `${kustomization}  - path: security-context.yaml\n` },
      { path, before: null, after },
    ],
  };
}

/** Tags added from Tag Coverage: the AWS provider's default_tags, the Dockerfile's OCI labels, or an Assist run. */
export function tagsChange(resource: TaggedResource, repository: string, path: string): { change: CodeChange; method: 'editor' | 'assist'; notes: string[] } {
  if (resource.target === 'image') {
    const before = `FROM node:20-alpine\nWORKDIR /app\nCOPY . .\nRUN npm ci --omit=dev\nCMD ["node", "server.js"]\n`;
    const after = before.replace(
      'FROM node:20-alpine\n',
      `FROM node:20-alpine\nARG GIT_COMMIT=unknown\nLABEL org.opencontainers.image.source="https://${repository}" \\\n      org.opencontainers.image.revision="$GIT_COMMIT"\n`,
    );
    const file = path ? `${path}/Dockerfile` : 'Dockerfile';
    return {
      method: 'editor',
      change: { title: `Label ${imageBase(resource.name)} with its source`, description: `Adds the OCI \`source\` and \`revision\` labels to \`${file}\`.`, files: [{ path: file, before, after }] },
      notes: ['Build with `--build-arg GIT_COMMIT=$(git rev-parse HEAD)` so the revision label carries the commit.'],
    };
  }
  const file = `${path ? `${path}/` : ''}providers.tf`;
  if (resource.provider === 'aws') {
    const before = `provider "aws" {\n  region = "${resource.region ?? 'eu-west-1'}"\n}\n`;
    const after = `provider "aws" {\n  region = "${resource.region ?? 'eu-west-1'}"\n\n  default_tags {\n    tags = {\n      managed-by = "terraform"\n      tf_repo    = "${repository}"\n      tf_path    = "${path}"\n    }\n  }\n}\n`;
    return {
      method: 'editor',
      change: {
        title: `Tag ${path || 'the root module'} with its source`,
        description: `Adds \`managed-by\`, \`tf_repo\` and \`tf_path\` to the AWS provider’s \`default_tags\`, so every resource this module creates names \`${repository}\` and \`${path}\`.`,
        files: [{ path: file, before, after }],
      },
      notes: ['`terraform plan` shows tag-only changes on every resource of this root module; apply it so the resources carry their source.'],
    };
  }
  const before = `locals {\n  location = "westeurope"\n}\n`;
  const after = `locals {\n  location = "westeurope"\n\n  default_tags = {\n    managed-by = "terraform"\n    tf_repo    = "${repository}"\n    tf_path    = "${path}"\n  }\n}\n`;
  return {
    method: 'assist',
    change: {
      title: `Tag ${path || 'the root module'} with its source`,
      description: 'Adds a `default_tags` local and merges it into the tags of every resource of the module (the azurerm provider has no default tags).',
      files: [{ path: `${path ? `${path}/` : ''}locals.tf`, before, after }],
    },
    notes: [],
  };
}
