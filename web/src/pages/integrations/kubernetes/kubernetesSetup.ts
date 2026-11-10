// Integrations → Kubernetes → Install: the official Datadog chart's Helm values or the Datadog Operator's
// DatadogAgent (Agent + Cluster Agent), or the observecraft-agent chart (OpenTelemetry Collector), pointed at this
// install's intake. deploy/examples/datadog is this output for the in-cluster intake (scripts/datadog-examples.mjs).

export type KubeChart = 'datadog' | 'datadog-operator' | 'opentelemetry';

export interface KubeInstallOptions {
  clusterName: string;
  /** The intake's URL as the cluster reaches it, e.g. https://intake.observecraft.example.com. */
  intakeUrl: string;
  logs: boolean;
  apm: boolean;
  /** The Cluster Agent's workload autoscaling (Datadog Agent only). */
  autoscaling: boolean;
  /** Live containers (the process agent's `/api/v1/container`). */
  containers: boolean;
  /** Live processes (`/api/v1/collector`). */
  processes: boolean;
  /** Software bills of materials of hosts and container images (Cloud Security's vulnerabilities). */
  sbom: boolean;
  /** Workload Protection: the runtime security module (system-probe + security-agent) and its events. */
  workloadProtection: boolean;
  /** Cloud Network Monitoring: system-probe's connections and DNS. */
  networkMonitoring: boolean;
}

/** deploy/examples/datadog: the in-cluster intake, with what the examples have always turned on. */
export const EXAMPLE_OPTIONS: KubeInstallOptions = {
  clusterName: 'my-cluster',
  intakeUrl: 'http://observecraft-intake.observecraft.svc:4318',
  logs: true,
  apm: true,
  autoscaling: false,
  containers: true,
  processes: false,
  sbom: true,
  workloadProtection: true,
  networkMonitoring: false,
};

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

/** An environment variable of the Agents, or a comment line above the next one. */
type EnvLine = [name: string, value: string] | string;

/** The Agents' environment: every product's intake URL, and the options' switches. */
function agentEnv(o: KubeInstallOptions): EnvLine[] {
  const intake = intakeEndpoints(o.intakeUrl);
  return [
    ['DD_APM_DD_URL', intake.url],
    ['DD_APM_PROFILING_DD_URL', intake.profiles],
    ['DD_ORCHESTRATOR_EXPLORER_ORCHESTRATOR_DD_URL', intake.url],
    "# Live containers and processes (and Cloud Network Monitoring's connections) go to the process intake.",
    ['DD_PROCESS_CONFIG_PROCESS_DD_URL', intake.url],
    ['DD_LOGS_CONFIG_LOGS_DD_URL', intake.logs],
    ['DD_LOGS_CONFIG_FORCE_USE_HTTP', 'true'],
    ...(intake.tls ? [] : ([['DD_LOGS_CONFIG_LOGS_NO_SSL', 'true']] as EnvLine[])),
    '# Stack traces and other multi-line messages as one log instead of one log per line: the Agent',
    "# groups lines that don't start a new log (a timestamp it recognises) with the line before. For an",
    "# application it doesn't get right, a rule on its pod (the pattern of a line that starts a log):",
    `#   ad.datadoghq.com/<container>.logs: '[{"source":"java","service":"order-service",`,
    `#     "log_processing_rules":[{"type":"multi_line","name":"new_log","pattern":"\\\\d{4}-\\\\d{2}-\\\\d{2}"}]}]'`,
    ['DD_LOGS_CONFIG_AUTO_MULTI_LINE_DETECTION', 'true'],
    "# Tracers' telemetry (the libraries each service loads) feeds Code Security.",
    ['DD_APM_TELEMETRY_ENABLED', 'true'],
    ['DD_APM_TELEMETRY_DD_URL', intake.url],
    ['DD_INSTRUMENTATION_TELEMETRY_ENABLED', 'false'],
    ...(o.sbom
      ? ([
          "# Software bills of materials of hosts and container images feed Cloud Security's vulnerabilities.",
          ['DD_SBOM_ENABLED', 'true'],
          ['DD_SBOM_HOST_ENABLED', 'true'],
          ['DD_SBOM_CONTAINER_IMAGE_ENABLED', 'true'],
          ['DD_SBOM_DD_URL', intake.url],
        ] as EnvLine[])
      : []),
    ...(o.workloadProtection
      ? ([
          "# Workload Protection's events (the runtime security track of the logs pipeline).",
          ['DD_RUNTIME_SECURITY_CONFIG_ENDPOINTS_LOGS_DD_URL', intake.logs],
          ...(intake.tls ? [] : ([['DD_RUNTIME_SECURITY_CONFIG_ENDPOINTS_LOGS_NO_SSL', 'true']] as EnvLine[])),
        ] as EnvLine[])
      : []),
    "# Products that would send to Datadog's own intakes (not to ObserveCraft): keep them off so nothing",
    '# leaves for Datadog.',
    ['DD_PROCESS_CONFIG_PROCESS_DISCOVERY_ENABLED', 'false'],
    ['DD_CONTAINER_IMAGE_ENABLED', 'false'],
    ['DD_CONTAINER_LIFECYCLE_ENABLED', 'false'],
  ];
}

const RECOMMENDER: EnvLine[] = ["# ObserveCraft's recommender for autoscalers annotated with it (off by default).", ['DD_AUTOSCALING_WORKLOAD_EXTERNAL_RECOMMENDER_ENABLED', 'true']];

const envYaml = (vars: EnvLine[], indent: string) => vars.map((v) => (typeof v === 'string' ? `${indent}${v}` : `${indent}- { name: ${v[0]}, value: ${q(v[1])} }`)).join('\n');

/** The node Agent's env as a YAML anchor, and the Cluster Agent's: the same, plus the recommender with autoscaling. */
function envBlocks(o: KubeInstallOptions, indent: string): { node: string[]; cluster: string[] } {
  const env = agentEnv(o);
  return {
    node: [`${indent}env: &env`, envYaml(env, `${indent}  `)],
    cluster: o.autoscaling ? [`${indent}env:`, envYaml([...env, ...RECOMMENDER], `${indent}  `)] : [`${indent}env: *env`],
  };
}

const AUTOSCALING_NOTE = [
  '# Workload autoscaling (Kubernetes → Autoscaling): the Cluster Agent applies DatadogPodAutoscalers and needs',
  '# Remote Configuration enabled; with the site above it never reaches Datadog.',
];

export function datadogValues(o: KubeInstallOptions): string {
  const intake = intakeEndpoints(o.intakeUrl);
  const env = envBlocks(o, '  ');
  const lines = [
    '# Official Datadog Helm chart (Agent + Cluster Agent), sending to ObserveCraft instead of Datadog.',
    "# Applications keep Datadog's tracing libraries and DogStatsD clients unchanged.",
    'datadog:',
    '  apiKeyExistingSecret: datadog-secret',
    `  clusterName: ${q(o.clusterName)}`,
    '  # Not a Datadog site: anything left unconfigured has nowhere to send, so no data reaches Datadog.',
    '  site: observecraft.invalid',
    '  # Clusters whose kubelet serves a self-signed certificate (kind, minikube, some on-prem setups):',
    '  # kubelet: { tlsVerify: false }',
    '  # Metrics, metadata, service checks, distributions and the API key check.',
    `  dd_url: ${q(intake.url)}`,
    '  logs:',
    `    enabled: ${o.logs}`,
    `    containerCollectAll: ${o.logs}`,
    '  apm:',
    `    portEnabled: ${o.apm}`,
    "  # DogStatsD on each node's IP (DD_AGENT_HOST=status.hostIP in your pods).",
    '  dogstatsd:',
    '    useHostPort: true',
    '  orchestratorExplorer:',
    '    enabled: true',
    '  # Live containers and processes (Infrastructure → Containers, Processes).',
    '  processAgent:',
    `    containerCollection: ${o.containers}`,
    `    processCollection: ${o.processes}`,
    '  # Workload Protection: the runtime security module (system-probe + security-agent) and its events.',
    '  # Agent rules come from the policy files mounted in the security agent (Workload Protection →',
    "  # Policies → Download), since ObserveCraft doesn't serve Remote Configuration.",
    '  securityAgent:',
    '    runtime:',
    `      enabled: ${o.workloadProtection}`,
    "  # Cloud Network Monitoring: system-probe's connections and DNS.",
    '  networkMonitoring:',
    `    enabled: ${o.networkMonitoring}`,
    ...(o.autoscaling
      ? [...AUTOSCALING_NOTE.map((l) => `  ${l}`), '  autoscaling:', '    workload:', '      enabled: true', '  remoteConfiguration:', '    enabled: true']
      : ["  # Products ObserveCraft doesn't have yet.", '  remoteConfiguration:', '    enabled: false']),
    ...env.node,
    'clusterAgent:',
    '  enabled: true',
    ...(o.autoscaling ? ['  # Vertical scaling sets pod requests through the admission controller.', '  admissionController:', '    enabled: true'] : []),
    ...env.cluster,
  ];
  return lines.join('\n') + '\n';
}

/** The Datadog Operator's DatadogAgent with the same choices. */
export function datadogOperatorManifest(o: KubeInstallOptions): string {
  const intake = intakeEndpoints(o.intakeUrl);
  const env = envBlocks(o, '      ');
  const lines = [
    "# The Datadog Operator's DatadogAgent (Agent + Cluster Agent), sending to ObserveCraft instead of Datadog.",
    'apiVersion: datadoghq.com/v2alpha1',
    'kind: DatadogAgent',
    'metadata:',
    '  name: datadog',
    '  namespace: datadog',
    'spec:',
    '  global:',
    `    clusterName: ${q(o.clusterName)}`,
    '    # Not a Datadog site: anything left unconfigured has nowhere to send, so no data reaches Datadog.',
    '    site: observecraft.invalid',
    '    # Clusters whose kubelet serves a self-signed certificate (kind, some on-prem setups):',
    '    # kubelet: { tlsVerify: false }',
    '    credentials:',
    '      apiSecret:',
    '        secretName: datadog-secret',
    '        keyName: api-key',
    '    # Metrics, metadata, service checks, distributions and the API key check.',
    '    endpoint:',
    `      url: ${q(intake.url)}`,
    '  features:',
    "    # APM (8126) and DogStatsD (8125) on each node's IP: DD_AGENT_HOST=status.hostIP in your pods.",
    '    apm:',
    `      enabled: ${o.apm}`,
    '      hostPortConfig:',
    '        enabled: true',
    '    dogstatsd:',
    '      hostPortConfig:',
    '        enabled: true',
    '    logCollection:',
    `      enabled: ${o.logs}`,
    `      containerCollectAll: ${o.logs}`,
    '    orchestratorExplorer:',
    '      enabled: true',
    '    # Live containers and processes (Infrastructure → Containers, Processes).',
    '    liveContainerCollection:',
    `      enabled: ${o.containers}`,
    '    liveProcessCollection:',
    `      enabled: ${o.processes}`,
    '    # Workload Protection: the runtime security module and its events. Agent rules come from the',
    '    # policy files mounted in the security agent (Workload Protection → Policies → Download).',
    '    cws:',
    `      enabled: ${o.workloadProtection}`,
    "    # Cloud Network Monitoring: system-probe's connections and DNS.",
    '    npm:',
    `      enabled: ${o.networkMonitoring}`,
    "    # Sends to Datadog's own intake.",
    '    processDiscovery:',
    '      enabled: false',
    ...(o.autoscaling
      ? [
          ...AUTOSCALING_NOTE.map((l) => `    ${l}`),
          '    autoscaling:',
          '      workload:',
          '        enabled: true',
          '    remoteConfiguration:',
          '      enabled: true',
          '    admissionController:',
          '      enabled: true',
        ]
      : ["    # Products ObserveCraft doesn't have yet.", '    remoteConfiguration:', '      enabled: false']),
    '  override:',
    '    nodeAgent:',
    ...env.node,
    '    clusterAgent:',
    ...env.cluster,
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
  if (chart === 'datadog-operator')
    return [
      'helm repo add datadog https://helm.datadoghq.com && helm repo update',
      'helm install datadog-operator datadog/datadog-operator -n datadog --create-namespace',
      'kubectl -n datadog create secret generic datadog-secret --from-literal api-key=<INGEST_API_KEY>',
      'kubectl apply -f datadog-agent.yaml',
    ];
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
