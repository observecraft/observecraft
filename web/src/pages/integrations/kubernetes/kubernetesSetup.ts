// Integrations → Kubernetes → Install: Helm values for the official Datadog chart (Agent + Cluster
// Agent, the same settings as deploy/examples/datadog/values.yaml) or the observecraft-agent chart
// (OpenTelemetry Collector), pointed at this install's intake.

export type KubeChart = 'datadog' | 'opentelemetry';

export interface KubeInstallOptions {
  clusterName: string;
  /** The intake's URL as the cluster reaches it, e.g. https://intake.observecraft.example.com. */
  intakeUrl: string;
  logs: boolean;
  apm: boolean;
  /** The Cluster Agent's workload autoscaling (Datadog chart only). */
  autoscaling: boolean;
}

export interface IntakeEndpoints {
  url: string;
  /** host:port, as the Agent's logs pipeline takes it. */
  logs: string;
  profiles: string;
  tls: boolean;
}

export function intakeEndpoints(raw: string): IntakeEndpoints {
  const url = raw.trim().replace(/\/+$/, '');
  let host = url.replace(/^https?:\/\//, '');
  const tls = !url.startsWith('http://');
  if (!/:\d+$/.test(host)) host = `${host}:${tls ? 443 : 80}`;
  return { url, logs: host, profiles: `${url}/api/v2/profile`, tls };
}

const q = (s: string) => JSON.stringify(s);

export function datadogValues(o: KubeInstallOptions): string {
  const intake = intakeEndpoints(o.intakeUrl);
  const env: [string, string][] = [
    ['DD_APM_DD_URL', intake.url],
    ['DD_APM_PROFILING_DD_URL', intake.profiles],
    ['DD_ORCHESTRATOR_EXPLORER_ORCHESTRATOR_DD_URL', intake.url],
    ['DD_PROCESS_CONFIG_PROCESS_DD_URL', intake.url],
    ['DD_LOGS_CONFIG_LOGS_DD_URL', intake.logs],
    ['DD_LOGS_CONFIG_FORCE_USE_HTTP', 'true'],
    ...(intake.tls ? [] : ([['DD_LOGS_CONFIG_LOGS_NO_SSL', 'true']] as [string, string][])),
    ['DD_APM_TELEMETRY_ENABLED', 'true'],
    ['DD_APM_TELEMETRY_DD_URL', intake.url],
    ['DD_INSTRUMENTATION_TELEMETRY_ENABLED', 'false'],
    ['DD_SBOM_ENABLED', 'true'],
    ['DD_SBOM_CONTAINER_IMAGE_ENABLED', 'true'],
    ['DD_SBOM_DD_URL', intake.url],
    // Products that would send to Datadog's own intakes: off, so nothing leaves for Datadog.
    ['DD_PROCESS_CONFIG_CONTAINER_COLLECTION_ENABLED', 'false'],
    ['DD_PROCESS_CONFIG_PROCESS_DISCOVERY_ENABLED', 'false'],
    ['DD_CONTAINER_IMAGE_ENABLED', 'false'],
    ['DD_CONTAINER_LIFECYCLE_ENABLED', 'false'],
  ];
  const envYaml = (vars: [string, string][], indent: string) => vars.map(([name, value]) => `${indent}- { name: ${name}, value: ${q(value)} }`).join('\n');
  // The Cluster Agent calls ObserveCraft's recommender for autoscalers annotated with it (off by default).
  const clusterEnv: [string, string][] = o.autoscaling ? [...env, ['DD_AUTOSCALING_WORKLOAD_EXTERNAL_RECOMMENDER_ENABLED', 'true']] : env;
  const lines = [
    '# Official Datadog Helm chart (Agent + Cluster Agent), sending to ObserveCraft.',
    '# Applications keep the Datadog tracing libraries and DogStatsD clients unchanged.',
    'datadog:',
    '  apiKeyExistingSecret: datadog-secret',
    `  clusterName: ${q(o.clusterName)}`,
    '  # Not a Datadog site: anything not sent to ObserveCraft has nowhere to go.',
    '  site: observecraft.invalid',
    `  dd_url: ${q(intake.url)}`,
    '  # Clusters whose kubelet serves a self-signed certificate (kind, minikube, some on-prem setups):',
    '  # kubelet: { tlsVerify: false }',
    '  logs:',
    `    enabled: ${o.logs}`,
    `    containerCollectAll: ${o.logs}`,
    '  apm:',
    `    portEnabled: ${o.apm}`,
    '  dogstatsd:',
    '    useHostPort: true',
    '  orchestratorExplorer:',
    '    enabled: true',
    '  processAgent:',
    '    processCollection: false',
    ...(o.autoscaling
      ? [
          '  # Workload autoscaling: the Cluster Agent applies DatadogPodAutoscalers. It requires Remote',
          '  # Configuration to be enabled; with the site above, Remote Configuration never reaches Datadog.',
          '  autoscaling:',
          '    workload:',
          '      enabled: true',
          '  remoteConfiguration:',
          '    enabled: true',
        ]
      : ['  remoteConfiguration:', '    enabled: false']),
    '  env:',
    envYaml(env, '    '),
    'clusterAgent:',
    '  enabled: true',
    ...(o.autoscaling ? ['  # Vertical scaling sets pod requests through the admission controller.', '  admissionController:', '    enabled: true'] : []),
    '  env:',
    envYaml(clusterEnv, '    '),
  ];
  return lines.join('\n') + '\n';
}

export function otelValues(o: KubeInstallOptions): string {
  return [
    '# observecraft-agent chart: the OpenTelemetry Collector as node and cluster agents.',
    `endpoint: ${q(intakeEndpoints(o.intakeUrl).url)}`,
    'existingSecret: observecraft-agent',
    `clusterName: ${q(o.clusterName)}`,
    'node:',
    '  logs:',
    `    enabled: ${o.logs}`,
    '  datadog:',
    `    traces: ${o.apm}`,
    '',
  ].join('\n');
}

export function helmCommands(chart: KubeChart): string[] {
  return chart === 'datadog'
    ? [
        'kubectl create namespace datadog',
        'kubectl -n datadog create secret generic datadog-secret --from-literal api-key=<INGEST_API_KEY>',
        'helm repo add datadog https://helm.datadoghq.com && helm repo update',
        'helm install datadog datadog/datadog -n datadog -f values.yaml',
      ]
    : [
        'kubectl create namespace observecraft',
        'kubectl -n observecraft create secret generic observecraft-agent --from-literal api-key=<INGEST_API_KEY>',
        'helm install observecraft-agent oci://ghcr.io/observecraft/charts/observecraft-agent -n observecraft -f values.yaml',
      ];
}
