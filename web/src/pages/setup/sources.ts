// Setup guides (Logs → Add a Log Source, Infrastructure → Add Resources): how each kind of sender reaches
// ObserveCraft. Snippets follow deploy/examples and the charts' values; {{intake}}, {{intakeHost}} and {{apiKey}}
// are filled in for this install; {{logsNoSsl}} is true for a plain-HTTP intake.

export type GuideKind = 'logs' | 'infrastructure';

export type CategoryName = 'Server' | 'Container' | 'Cloud' | 'Other';

export interface SetupCategory {
  name: CategoryName;
  /** What the category covers, under its name. */
  examples: string;
}

const CATEGORIES: SetupCategory[] = [
  { name: 'Server', examples: 'Hosts and VMs, with an agent' },
  { name: 'Container', examples: 'Docker, Kubernetes' },
  { name: 'Cloud', examples: 'Amazon ECS' },
  { name: 'Other', examples: 'HTTP API, log shippers' },
];

export interface SetupStep {
  title: string;
  text?: string;
  /** A command or config file, with placeholders. */
  code?: string;
}

export interface SetupSource {
  id: string;
  name: string;
  category: CategoryName;
  /** What it collects, shown under the name. */
  summary: string;
  kinds: GuideKind[];
  steps: SetupStep[];
}

const DATADOG_HELM_VALUES = `# values.yaml: the official Datadog chart, sending to ObserveCraft (full file: deploy/examples/datadog/values.yaml)
datadog:
  apiKeyExistingSecret: datadog-secret
  clusterName: my-cluster
  site: observecraft.invalid   # nothing is sent to Datadog
  dd_url: {{intake}}
  logs:
    enabled: true
    containerCollectAll: true
  apm:
    portEnabled: true
  orchestratorExplorer:
    enabled: true
  env:
    - { name: DD_APM_DD_URL, value: {{intake}} }
    - { name: DD_ORCHESTRATOR_EXPLORER_ORCHESTRATOR_DD_URL, value: {{intake}} }
    - { name: DD_LOGS_CONFIG_LOGS_DD_URL, value: {{intakeHost}} }
    - { name: DD_LOGS_CONFIG_FORCE_USE_HTTP, value: "true" }
    - { name: DD_LOGS_CONFIG_LOGS_NO_SSL, value: "{{logsNoSsl}}" }
clusterAgent:
  enabled: true`;

export const SOURCES: SetupSource[] = [
  {
    id: 'kubernetes-datadog',
    name: 'Kubernetes with the Datadog Agent',
    category: 'Container',
    summary: 'Container logs, node and pod metrics, Kubernetes state, events and traces, from the official Datadog Helm chart.',
    kinds: ['logs', 'infrastructure'],
    steps: [
      { title: 'Store your API key in the cluster', code: 'kubectl create namespace datadog\nkubectl -n datadog create secret generic datadog-secret --from-literal api-key={{apiKey}}' },
      { title: 'Point the chart at ObserveCraft', text: 'Keep the rest of your Datadog values; these settings send everything to this install.', code: DATADOG_HELM_VALUES },
      { title: 'Install the chart', code: 'helm repo add datadog https://helm.datadoghq.com\nhelm install datadog datadog/datadog -n datadog -f values.yaml --set datadog.clusterName=my-cluster' },
    ],
  },
  {
    id: 'kubernetes-otel',
    name: 'Kubernetes with OpenTelemetry',
    category: 'Container',
    summary: 'The observecraft-agent chart: the OpenTelemetry Collector as a node and cluster agent (logs, metrics, traces, Kubernetes state).',
    kinds: ['logs', 'infrastructure'],
    steps: [
      {
        title: 'Install the agent chart',
        code: 'helm install observecraft-agent oci://ghcr.io/observecraft/charts/observecraft-agent -n observecraft --create-namespace \\\n  --set endpoint={{intake}} --set clusterName=my-cluster --set apiKey={{apiKey}}',
      },
      { title: 'Tag your workloads', text: 'Set the pod labels tags.datadoghq.com/service, env and version (or OTEL_SERVICE_NAME) so logs, metrics and traces share their service.' },
    ],
  },
  {
    id: 'host-datadog',
    name: 'Host with the Datadog Agent',
    category: 'Server',
    summary: 'A VM or bare-metal host: system metrics, log files, processes and the services running on it.',
    kinds: ['logs', 'infrastructure'],
    steps: [
      { title: 'Install the Agent', text: "Use Datadog's install script or package for your OS, then edit its configuration." },
      {
        title: 'Send to ObserveCraft (datadog.yaml)',
        code: 'api_key: {{apiKey}}\nsite: observecraft.invalid\ndd_url: {{intake}}\nlogs_enabled: true\nlogs_config:\n  logs_dd_url: {{intakeHost}}\n  force_use_http: true\n  logs_no_ssl: {{logsNoSsl}}\napm_config:\n  apm_dd_url: {{intake}}\nprocess_config:\n  process_dd_url: {{intake}}',
      },
      {
        title: 'Collect log files',
        text: 'Add a conf.d/<name>.d/conf.yaml with a logs section per file, then restart the Agent.',
        code: 'logs:\n  - type: file\n    path: /var/log/myapp/*.log\n    service: myapp\n    source: myapp',
      },
    ],
  },
  {
    id: 'host-otel',
    name: 'Host with OpenTelemetry',
    category: 'Server',
    summary: 'The OpenTelemetry Collector on a host or Docker host: host metrics, log files and a local endpoint for applications.',
    kinds: ['logs', 'infrastructure'],
    steps: [
      { title: 'Get the configuration', text: 'Download deploy/examples/otel-host/collector.yaml from the ObserveCraft repository.' },
      {
        title: 'Run the Collector',
        code: 'docker run -d --name otel-collector --network host --pid host \\\n  -v /:/hostfs:ro -v /var/log:/var/log:ro -v otel-collector:/var/lib/observecraft-agent \\\n  -v $PWD/collector.yaml:/etc/otelcol-contrib/config.yaml:ro \\\n  -e OBSERVECRAFT_ENDPOINT={{intake}} -e OBSERVECRAFT_API_KEY={{apiKey}} \\\n  -e OBSERVECRAFT_HOSTFS=/hostfs -e HOST_PROC_MOUNTINFO=/hostfs/proc/1 \\\n  otel/opentelemetry-collector-contrib:0.163.2',
      },
    ],
  },
  {
    id: 'docker',
    name: 'Docker containers',
    category: 'Container',
    summary: "Container logs and metrics on a Docker host, through the host's Collector or Datadog Agent.",
    kinds: ['logs', 'infrastructure'],
    steps: [
      { title: 'Run an agent on the host', text: 'Use "Host with OpenTelemetry" or "Host with the Datadog Agent"; both read container logs from the Docker log files.' },
      { title: 'Label your containers', code: 'docker run -l com.datadoghq.tags.service=checkout -l com.datadoghq.tags.env=prod …' },
    ],
  },
  {
    id: 'aws-ecs',
    name: 'Amazon ECS',
    category: 'Cloud',
    summary: 'ECS tasks on Fargate or EC2: logs through FireLens and traces and metrics through the ADOT Collector sidecar.',
    kinds: ['logs', 'infrastructure'],
    steps: [
      { title: 'Add the sidecars to your task definitions', text: 'Start from deploy/examples/ecs (firelens-task-definition.json and adot-sidecar-container.json) and set the intake and API key.' },
      { title: 'Intake and key', code: 'OBSERVECRAFT_ENDPOINT={{intake}}\nOBSERVECRAFT_API_KEY={{apiKey}}' },
    ],
  },
  {
    id: 'http-api',
    name: 'HTTP API',
    category: 'Other',
    summary: "Send JSON logs from anything that speaks HTTP: Datadog's logs API format, one event or a batch.",
    kinds: ['logs'],
    steps: [
      {
        title: 'Send a log',
        code: `curl -X POST {{intake}}/api/v2/logs \\\n  -H "DD-API-KEY: {{apiKey}}" -H "content-type: application/json" \\\n  -d '[{"message":"order 1042 paid","service":"checkout","ddsource":"myapp","status":"info","ddtags":"env:prod"}]'`,
      },
      { title: 'From your code', text: 'Any Datadog logging library or HTTP log shipper (Vector, Fluent Bit, Logstash) works: point its Datadog logs output at the intake with your API key.' },
    ],
  },
];

/** The sources a guide lists. */
export function sourcesFor(kind: GuideKind): SetupSource[] {
  return SOURCES.filter((s) => s.kinds.includes(kind));
}

/** The categories a guide shows, in order, each with at least one source. */
export function categoriesFor(kind: GuideKind): SetupCategory[] {
  const sources = sourcesFor(kind);
  return CATEGORIES.filter((c) => sources.some((s) => s.category === c.name));
}

/** The intake as host:port (agents that take one), from its URL. */
function intakeHost(intake: string): string {
  const url = new URL(intake);
  return `${url.hostname}:${url.port || (url.protocol === 'https:' ? '443' : '80')}`;
}

/** A snippet with this install's intake and the API key placeholder. */
export function fill(template: string, values: { intake: string; apiKey: string }): string {
  const noSsl = String(new URL(values.intake).protocol === 'http:');
  return template.replaceAll('{{logsNoSsl}}', noSsl).replaceAll('{{intakeHost}}', intakeHost(values.intake)).replaceAll('{{intake}}', values.intake).replaceAll('{{apiKey}}', values.apiKey);
}
