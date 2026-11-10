// Mock logs of the Kubernetes tools whose integration dashboards the mock shows (Vault, Gatekeeper, Argo CD,
// cert-manager), as the Datadog Agent collects them from their pods and the integrations' pipelines parse them:
// Vault's audit device (`source:vault`, @auth.* / @request.*), Gatekeeper's admission and audit events, Argo CD's
// controllers and cert-manager's. Tagged like the dashboards' metrics, on the demo's clusters and hosts.
import type { JsonValue, LogEvent, LogStatus } from '../types';
import type { Rand } from './generator';

const MIN = 60_000;
const HOUR = 60 * MIN;
const HOSTS = ['i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003', 'i-0a1b2c3d4e5f60006'];
const CLUSTERS = ['shop-prod-eks', 'shop-prod-eks', 'shop-prod-eks', 'shop-qa-eks'];

const pick = <T>(r: Rand, a: readonly T[]): T => a[Math.floor(r() * a.length)];
/** A Vault audit HMAC (`hmac-sha256:…`), stable per seed. */
const hmac = (seed: string) =>
  `hmac-sha256:${[...seed]
    .reduce((h, c) => (Math.imul(h, 31) + c.charCodeAt(0)) >>> 0, 7)
    .toString(16)
    .padStart(8, '0')}${'5e1f0c2d4b3a69788f'.slice(seed.length % 9)}`;

/** Vault's clients: display name, token type, TTL (seconds), address, the paths they read and write. */
const VAULT_CLIENTS: [name: string, tokenType: string, ttl: number, ip: string, paths: [mount: string, path: string, operation: string][]][] = [
  [
    'kubernetes-storefront-storefront-api',
    'service',
    3600,
    '10.0.12.34',
    [
      ['kv', 'secret/data/storefront/db', 'read'],
      ['kv', 'secret/data/storefront/session-key', 'read'],
    ],
  ],
  [
    'kubernetes-checkout-order-service',
    'service',
    3600,
    '10.0.14.71',
    [
      ['kv', 'secret/data/checkout/payments', 'read'],
      ['transit', 'transit/encrypt/orders', 'update'],
    ],
  ],
  ['kubernetes-billing-billing', 'batch', 1800, '10.0.16.9', [['database', 'database/creds/billing-rw', 'read']]],
  [
    'approle-ci-deployer',
    'service',
    2764800,
    '10.0.40.5',
    [
      ['kv', 'secret/data/catalog/search-key', 'create'],
      ['kv', 'secret/metadata/catalog', 'list'],
    ],
  ],
  [
    'oidc-alice@example.com',
    'service',
    2764800,
    '203.0.113.24',
    [
      ['pki', 'pki_int/issue/storefront-example-com', 'update'],
      ['sys', 'sys/policies/acl/storefront', 'update'],
    ],
  ],
  ['token-external-secrets', 'service', 2764800, '10.0.22.17', [['kv', 'secret/data/billing/stripe', 'read']]],
];

function vaultLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (let i = 0; i < 900; i++) {
    const ts = Math.round(now - Math.pow(r(), 1.4) * 24 * HOUR);
    const [name, tokenType, ttl, ip, paths] = pick(r, VAULT_CLIENTS);
    const [mount, path, operation] = pick(r, paths);
    const node = Math.floor(r() * 3);
    const denied = r() < 0.04;
    const type = r() < 0.5 ? 'request' : 'response';
    const status: LogStatus = denied && type === 'response' ? 'error' : 'info';
    const accessor = hmac(`${name}-accessor`);
    const attributes: Record<string, JsonValue> = {
      type,
      auth: { client_token: hmac(`${name}-token`), accessor, display_name: name, token_type: tokenType, token_ttl: ttl, policies: ['default', name.split('-').slice(1, 3).join('-')] },
      request: {
        id: `${(ts % 0xffffff).toString(16)}-${i}`,
        operation,
        mount_type: mount,
        path,
        remote_address: ip,
        client_token: hmac(`${name}-token`),
        client_token_accessor: accessor,
        namespace: { id: 'root' },
      },
      ...(denied && type === 'response' ? { error: '1 error occurred:\n\t* permission denied\n\n' } : {}),
    };
    out.push({
      id: `vault-${ts}-${i}`,
      timestamp: ts,
      status,
      service: 'vault',
      host: HOSTS[node],
      source: 'vault',
      message: `${type} ${operation} ${path} by ${name}${denied && type === 'response' ? ': permission denied' : ''}`,
      tags: { env: 'prod', vault_cluster: 'vault-prod', api_url: `https://vault-${node}.vault-internal:8200`, kube_cluster_name: 'shop-prod-eks', kube_namespace: 'vault', pod_name: `vault-${node}` },
      attributes,
    });
  }
  return out;
}

const CONSTRAINTS: [kind: string, name: string, resource: string][] = [
  ['K8sRequiredLabels', 'must-have-team-label', 'Deployment'],
  ['K8sContainerLimits', 'containers-must-have-limits', 'Pod'],
  ['K8sAllowedRepos', 'allowed-registries', 'Pod'],
  ['K8sPSPPrivilegedContainer', 'no-privileged-containers', 'Pod'],
];
const WORKLOADS = ['storefront-api', 'order-service', 'billing', 'catalog-service', 'inventory-service'];
const NAMESPACES = ['storefront', 'checkout', 'billing', 'catalog'];

function gatekeeperLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (let i = 0; i < 360; i++) {
    const ts = Math.round(now - r() * 24 * HOUR);
    const c = Math.floor(r() * 4);
    const [kind, constraint, resource] = pick(r, CONSTRAINTS);
    const workload = pick(r, WORKLOADS);
    const namespace = pick(r, NAMESPACES);
    const audit = r() < 0.5;
    const denied = !audit && r() < 0.3;
    const event = audit ? 'violation_audited' : denied ? 'violation' : 'admission';
    const msg = audit ? 'audit results for constraint' : denied ? 'denied admission' : 'admission request allowed';
    out.push({
      id: `gatekeeper-${ts}-${i}`,
      timestamp: ts,
      status: denied ? 'warn' : 'info',
      service: 'gatekeeper',
      host: HOSTS[c],
      source: 'gatekeeper',
      message: `${msg}: ${kind}/${constraint} on ${resource} ${namespace}/${workload}`,
      tags: { cluster_name: CLUSTERS[c], kube_cluster_name: CLUSTERS[c], kube_namespace: 'gatekeeper-system', kube_deployment: audit ? 'gatekeeper-audit' : 'gatekeeper-controller-manager' },
      attributes: {
        level: denied ? 'warning' : 'info',
        logger: audit ? 'controller' : 'webhook',
        process: audit ? 'audit' : 'admission',
        event_type: event,
        constraint_kind: kind,
        constraint_name: constraint,
        constraint_action: 'deny',
        resource_kind: resource,
        resource_namespace: namespace,
        resource_name: workload,
        request_username: 'system:serviceaccount:argocd:argocd-application-controller',
      },
    });
  }
  return out;
}

const ARGO: [service: string, level: LogStatus, message: (app: string) => string][] = [
  ['argocd-application-controller', 'info', (app) => `Reconciliation completed application=argocd/${app}`],
  ['argocd-application-controller', 'info', (app) => `Updated sync status: Synced -> OutOfSync application=argocd/${app}`],
  ['argocd-application-controller', 'info', (app) => `Sync operation to main succeeded application=argocd/${app}`],
  ['argocd-application-controller', 'warn', (app) => `Comparison error: rpc error: code = Unknown desc = manifest generation timed out application=argocd/${app}`],
  ['argocd-repo-server', 'info', () => 'git fetch origin --tags --force --prune completed repo=https://github.com/example-org/storefront-deploy'],
  ['argocd-server', 'info', (app) => `finished unary call with code OK grpc.method=Get grpc.service=application.ApplicationService application=${app}`],
];

function argocdLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (let i = 0; i < 360; i++) {
    const ts = Math.round(now - r() * 24 * HOUR);
    const [service, status, message] = pick(r, ARGO);
    const app = pick(r, WORKLOADS);
    const c = Math.floor(r() * 4);
    out.push({
      id: `argocd-${ts}-${i}`,
      timestamp: ts,
      status,
      service,
      host: HOSTS[c],
      source: 'argocd',
      message: message(app),
      tags: { kube_cluster_name: CLUSTERS[c], kube_namespace: 'argocd', kube_deployment: service },
      attributes: { level: status === 'warn' ? 'warning' : 'info', application: app, msg: message(app) },
    });
  }
  return out;
}

const CERT_MANAGER: [service: string, status: LogStatus, message: (cert: string) => string][] = [
  ['cert-manager', 'info', (cert) => `"Certificate is up to date and has not expired" certificate="storefront/${cert}"`],
  ['cert-manager', 'info', (cert) => `"Issued certificate" logger="cert-manager.controller" certificateRequest="storefront/${cert}-1"`],
  ['cert-manager', 'info', (cert) => `"Renewing certificate" certificate="storefront/${cert}"`],
  ['cert-manager', 'warn', (cert) => `"propagation check failed" err="DNS record for \\"${cert.replace(/-/g, '.')}\\" not yet propagated" resource_name="${cert}-1-0"`],
  ['cert-manager-webhook', 'info', () => '"Updated cert-manager TLS certificate" DNSNames=["cert-manager-webhook","cert-manager-webhook.cert-manager"]'],
  ['cert-manager-cainjector', 'info', () => '"Updated object" kind="validatingwebhookconfiguration" name="gatekeeper-validating-webhook-configuration"'],
];
const CERTS = ['storefront-example-com', 'api-example-com', 'checkout-example-com', 'internal-example-com'];

function certManagerLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (let i = 0; i < 240; i++) {
    const ts = Math.round(now - r() * 24 * HOUR);
    const [service, status, message] = pick(r, CERT_MANAGER);
    const c = Math.floor(r() * 4);
    out.push({
      id: `cert-manager-${ts}-${i}`,
      timestamp: ts,
      status,
      service,
      host: HOSTS[c],
      source: 'cert-manager',
      message: message(pick(r, CERTS)),
      tags: { cluster_name: CLUSTERS[c], kube_cluster_name: CLUSTERS[c], kube_namespace: 'cert-manager', kube_deployment: service },
      attributes: { level: status === 'warn' ? 'warning' : 'info', logger: `${service}.controller` },
    });
  }
  return out;
}

/** The four tools' logs over the last day. */
export function kubernetesToolLogs(r: Rand, now: number): LogEvent[] {
  return [...vaultLogs(r, now), ...gatekeeperLogs(r, now), ...argocdLogs(r, now), ...certManagerLogs(r, now)];
}
