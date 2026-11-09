// In-browser stand-in for the Integrations endpoints (see api/integrations/client.ts).
// Each catalog entry declares HOW it is collected; config files, metric lists and hosts are
// derived from that, so the Configure tab shows the collector config "Install" would apply.
// Generated configs are templates — verify field names against the pinned collector version.
import type { CollectedMetric, CollectorConfig, IntegrationDetail, IntegrationHost, IntegrationStatus, IntegrationSummary } from '../integrations/types';
import { rng } from './generator';
import { coreCatalog, type CoreIntegration } from '../integrations/catalog';
import { linkCore } from '../integrations/coreLink';
import { awsMetricPrefix } from '../integrations/detection';
import awsCatalog from '../../../../server/seeds/aws_integration.json';
import azureCatalog from '../../../../server/seeds/azure_integration.json';
import gcpCatalog from '../../../../server/seeds/gcp_integration.json';

const delay = <T>(value: T, ms = 100 + Math.random() * 150) => new Promise<T>((r) => setTimeout(() => r(value), ms));

// ---------------------------------------------------------------- categories

const FEATURED = [
  'AWS',
  'Azure',
  'Google Cloud',
  'Oracle',
  'AI/ML',
  'Alerting',
  'Automation',
  'Collaboration',
  'Configuration & Deployment',
  'Containers',
  'Data Stores',
  'Developer Tools',
  'Kubernetes',
  'Message Queues',
  'Network',
  'OS & System',
  'Security',
  'Testing',
];
const MORE = ['Caching', 'Cost Management', 'Incidents', 'Languages', 'Log Collection', 'Metrics', 'Notifications', 'Orchestration', 'Source Control', 'Tracing', 'Web', 'Windows'];

export function listCategories() {
  const all = new Set([...FEATURED, ...MORE, ...coreCatalog.integrations.flatMap((c) => c.categories)]);
  return delay({ featured: FEATURED, all: [...all].sort() }, 30);
}

// ---------------------------------------------------------------- collection specs

type M = [name: string, type: CollectedMetric['type'], description: string, unit?: string];

type Collect =
  | { kind: 'otel'; receiver: string; fields: string[]; metrics: M[] }
  | { kind: 'prometheus'; target: string; metrics: M[] }
  /** Collected from CloudWatch by the Amazon Web Services integration (`namespace` absent: the integration itself). */
  | { kind: 'cloud'; cloud: CloudId; service?: string; prefix: string }
  | { kind: 'otlp'; metrics: M[] }
  | { kind: 'notify'; settings: IntegrationDetail['settings'] }
  /** A Datadog Agent check from integrations-core, configured from its spec. */
  | { kind: 'agent' };

interface Entry {
  id: string;
  name: string;
  logo?: string;
  categories: string[];
  status?: IntegrationStatus;
  collect: Collect;
  /** Vector log source: "file:<glob>" | "kubernetes" | "journald" | "windows_event_log". */
  logs?: string;
  os?: string[];
  warning?: string;
  about?: string;
  /** The integrations-core entry for the same integration. */
  core?: CoreIntegration;
}

const REDIS: M[] = [
  ['redis.clients.connected', 'gauge', 'Client connections'],
  ['redis.memory.used', 'gauge', 'Memory used by Redis', 'By'],
  ['redis.commands.processed', 'count', 'Commands processed'],
  ['redis.keyspace.hits', 'count', 'Successful key lookups'],
  ['redis.keyspace.misses', 'count', 'Failed key lookups'],
  ['redis.keys.evicted', 'count', 'Keys evicted due to maxmemory'],
];
const APACHE: M[] = [
  ['apache.requests', 'count', 'Requests served'],
  ['apache.traffic', 'count', 'Bytes served', 'By'],
  ['apache.workers', 'gauge', 'Busy and idle workers'],
  ['apache.current_connections', 'gauge', 'Active connections'],
  ['apache.uptime', 'gauge', 'Server uptime', 's'],
];
const NGINX: M[] = [
  ['nginx.requests', 'count', 'Requests handled'],
  ['nginx.connections_accepted', 'count', 'Accepted client connections'],
  ['nginx.connections_current', 'gauge', 'Connections by state'],
];
const POSTGRES: M[] = [
  ['postgresql.backends', 'gauge', 'Active backends'],
  ['postgresql.commits', 'count', 'Committed transactions'],
  ['postgresql.rollbacks', 'count', 'Rolled back transactions'],
  ['postgresql.db_size', 'gauge', 'Database size', 'By'],
  ['postgresql.deadlocks', 'count', 'Deadlocks detected'],
];
const MONGO: M[] = [
  ['mongodb.connection.count', 'gauge', 'Connections by type'],
  ['mongodb.operation.count', 'count', 'Operations by type'],
  ['mongodb.memory.usage', 'gauge', 'Memory usage', 'By'],
  ['mongodb.document.operation.count', 'count', 'Document operations'],
];
const KAFKA: M[] = [
  ['kafka.brokers', 'gauge', 'Brokers in the cluster'],
  ['kafka.topic.partitions', 'gauge', 'Partitions per topic'],
  ['kafka.consumer_group.lag', 'gauge', 'Consumer group lag'],
  ['kafka.partition.current_offset', 'gauge', 'Current partition offset'],
];
const RABBIT: M[] = [
  ['rabbitmq.consumer.count', 'gauge', 'Consumers per queue'],
  ['rabbitmq.message.current', 'gauge', 'Messages in queue'],
  ['rabbitmq.message.published', 'count', 'Messages published'],
  ['rabbitmq.message.delivered', 'count', 'Messages delivered'],
];
const K8S: M[] = [
  ['k8s.pod.phase', 'gauge', 'Pod phase'],
  ['k8s.deployment.available', 'gauge', 'Available replicas per deployment'],
  ['k8s.node.condition', 'gauge', 'Node conditions'],
  ['k8s.container.restarts', 'gauge', 'Container restarts'],
  ['k8s.pod.cpu.utilization', 'gauge', 'Pod CPU utilization', '1'],
];
const HOST: M[] = [
  ['system.cpu.utilization', 'gauge', 'CPU utilization', '1'],
  ['system.memory.usage', 'gauge', 'Memory usage', 'By'],
  ['system.filesystem.usage', 'gauge', 'Filesystem usage', 'By'],
  ['system.network.io', 'count', 'Network bytes', 'By'],
];
const LLM: M[] = [
  ['gen_ai.client.operation.duration', 'distribution', 'LLM request latency', 's'],
  ['gen_ai.client.token.usage', 'distribution', 'Input/output tokens per request'],
  ['llm.request.count', 'count', 'Requests by model'],
  ['llm.request.errors', 'count', 'Failed requests by model'],
];
const RUNTIME = (lang: string): M[] => [
  [`process.runtime.${lang}.gc.collections.count`, 'count', 'Garbage collections'],
  [`process.runtime.${lang}.memory.heap`, 'gauge', 'Heap size', 'By'],
  [`process.runtime.${lang}.threads`, 'gauge', 'Threads'],
  [`process.runtime.${lang}.exceptions.count`, 'count', 'Exceptions thrown'],
];
const prom = (prefix: string, names: [string, CollectedMetric['type'], string][]): M[] => names.map(([n, t, d]) => [`${prefix}_${n}`, t, d]);

const NOTIFY_WEBHOOK = [{ key: 'webhook_url', label: 'Webhook URL', placeholder: 'https://hooks.example.com/…', secret: true }];

const CURATED: Entry[] = [
  // ---- autodetected
  {
    id: 'apache',
    name: 'Apache',
    categories: ['Web', 'Log Collection', 'Metrics'],
    status: 'detected',
    logs: 'file:/var/log/apache2/*.log',
    collect: { kind: 'otel', receiver: 'apache', fields: ['endpoint: http://localhost:80/server-status?auto'], metrics: APACHE },
  },
  {
    id: 'argo-rollouts',
    name: 'Argo Rollouts',
    logo: 'argo',
    categories: ['Kubernetes', 'Configuration & Deployment'],
    status: 'detected',
    collect: {
      kind: 'prometheus',
      target: 'argo-rollouts-metrics.argo-rollouts:8090',
      metrics: prom('rollout', [
        ['info', 'gauge', 'Rollout info and phase'],
        ['reconcile_count', 'count', 'Reconciliations'],
        ['events_total', 'count', 'Rollout events'],
      ]),
    },
  },
  {
    id: 'celery',
    name: 'Celery',
    categories: ['Message Queues', 'Languages'],
    status: 'detected',
    collect: {
      kind: 'prometheus',
      target: 'celery-flower:5555',
      metrics: prom('flower', [
        ['events_total', 'count', 'Task events by type'],
        ['worker_online', 'gauge', 'Online workers'],
        ['task_runtime_seconds', 'distribution', 'Task runtime'],
      ]),
    },
  },
  {
    id: 'envoy',
    name: 'Envoy',
    logo: 'envoyproxy',
    categories: ['Network', 'Web'],
    status: 'detected',
    collect: {
      kind: 'prometheus',
      target: 'localhost:9901/stats/prometheus',
      metrics: prom('envoy', [
        ['cluster_upstream_rq_total', 'count', 'Upstream requests'],
        ['cluster_upstream_cx_active', 'gauge', 'Active upstream connections'],
        ['http_downstream_rq_xx', 'count', 'Downstream responses by class'],
      ]),
    },
  },
  {
    id: 'gunicorn',
    name: 'Gunicorn',
    categories: ['Web', 'Languages'],
    status: 'detected',
    logs: 'file:/var/log/gunicorn/*.log',
    collect: {
      kind: 'otel',
      receiver: 'statsd',
      fields: ['endpoint: localhost:8125', 'aggregation_interval: 15s'],
      metrics: [
        ['gunicorn.requests', 'rate', 'Requests per second'],
        ['gunicorn.request.duration', 'distribution', 'Request duration', 'ms'],
        ['gunicorn.workers', 'gauge', 'Worker processes'],
      ],
    },
  },
  {
    id: 'keda',
    name: 'KEDA',
    categories: ['Kubernetes', 'Log Collection', 'Metrics', 'Security'],
    status: 'detected',
    about: 'KEDA is a Kubernetes-based event-driven autoscaler.',
    collect: {
      kind: 'prometheus',
      target: 'keda-operator-metrics-apiserver.keda:8080',
      metrics: prom('keda', [
        ['scaler_active', 'gauge', 'Active scalers'],
        ['scaler_metrics_value', 'gauge', 'Current metric value per scaler'],
        ['scaled_object_errors_total', 'count', 'ScaledObject errors'],
      ]),
    },
  },
  {
    id: 'cluster-autoscaler',
    name: 'Kubernetes Cluster Autoscaler',
    logo: 'kubernetes',
    categories: ['Kubernetes', 'Orchestration'],
    status: 'detected',
    collect: {
      kind: 'prometheus',
      target: 'cluster-autoscaler.kube-system:8085',
      metrics: prom('cluster_autoscaler', [
        ['nodes_count', 'gauge', 'Nodes by state'],
        ['unschedulable_pods_count', 'gauge', 'Unschedulable pods'],
        ['scaled_up_nodes_total', 'count', 'Scale-up events'],
      ]),
    },
  },
  {
    id: 'supervisord',
    name: 'Supervisord',
    categories: ['OS & System'],
    status: 'detected',
    collect: {
      kind: 'prometheus',
      target: 'localhost:9876',
      metrics: prom('supervisord', [
        ['up', 'gauge', 'Process is running'],
        ['start_time_seconds', 'gauge', 'Process start time'],
        ['exit_status', 'gauge', 'Last exit status'],
      ]),
    },
  },
  {
    id: 'tomcat',
    name: 'Tomcat',
    logo: 'apachetomcat',
    categories: ['Web', 'Languages'],
    status: 'detected',
    logs: 'file:/opt/tomcat/logs/*.log',
    collect: {
      kind: 'otel',
      receiver: 'jmx',
      fields: ['jar_path: /opt/opentelemetry-jmx-metrics.jar', 'endpoint: localhost:9010', 'target_system: tomcat'],
      metrics: [
        ['tomcat.sessions', 'gauge', 'Active sessions'],
        ['tomcat.errors', 'count', 'Request errors'],
        ['tomcat.request_count', 'count', 'Requests'],
        ['tomcat.threads', 'gauge', 'Threads by state'],
        ['tomcat.processing_time', 'count', 'Processing time', 'ms'],
      ],
    },
  },
  {
    id: 'velero',
    name: 'Velero',
    categories: ['Kubernetes', 'Data Stores'],
    status: 'detected',
    collect: {
      kind: 'prometheus',
      target: 'velero.velero:8085',
      metrics: prom('velero', [
        ['backup_success_total', 'count', 'Successful backups'],
        ['backup_failure_total', 'count', 'Failed backups'],
        ['backup_last_successful_timestamp', 'gauge', 'Last successful backup'],
      ]),
    },
  },
  {
    id: 'windows-cert-store',
    name: 'Windows Certificate Store',
    categories: ['Windows', 'Security', 'OS & System'],
    status: 'detected',
    os: ['Windows'],
    collect: {
      kind: 'prometheus',
      target: 'localhost:9182',
      metrics: prom('windows_certstore', [
        ['cert_expiry_seconds', 'gauge', 'Seconds until certificate expiry'],
        ['cert_count', 'gauge', 'Certificates per store'],
      ]),
    },
  },
  // ---- installed
  {
    id: 'coredns',
    name: 'CoreDNS',
    categories: ['Kubernetes', 'Network'],
    status: 'installed',
    collect: {
      kind: 'prometheus',
      target: 'kube-dns.kube-system:9153',
      metrics: prom('coredns', [
        ['dns_requests_total', 'count', 'DNS requests'],
        ['dns_responses_total', 'count', 'Responses by rcode'],
        ['dns_request_duration_seconds', 'distribution', 'Request duration'],
      ]),
    },
  },
  {
    id: 'redis',
    name: 'Redis',
    categories: ['Data Stores', 'Caching'],
    status: 'installed',
    logs: 'file:/var/log/redis/*.log',
    collect: { kind: 'otel', receiver: 'redis', fields: ['endpoint: ${env:REDIS_ENDPOINT}', 'password: ${env:REDIS_PASSWORD}'], metrics: REDIS },
  },
  { id: 'dotnet', name: '.NET Runtime Metrics', categories: ['Languages', 'Tracing'], status: 'installed', collect: { kind: 'otlp', metrics: RUNTIME('dotnet') } },
  {
    id: 'active-directory',
    name: 'Active Directory',
    categories: ['Windows', 'Security'],
    status: 'installed',
    os: ['Windows'],
    logs: 'windows_event_log',
    collect: {
      kind: 'otel',
      receiver: 'active_directory_ds',
      fields: [],
      metrics: [
        ['active_directory.ds.replication.network.io', 'count', 'Replication traffic', 'By'],
        ['active_directory.ds.thread.count', 'gauge', 'Threads in use'],
        ['active_directory.ds.bind.rate', 'rate', 'Binds per second'],
        ['active_directory.ds.ldap.search.rate', 'rate', 'LDAP searches per second'],
      ],
    },
  },
  {
    id: 'aws',
    name: 'Amazon Web Services',
    logo: 'amazonwebservices',
    categories: ['AWS', 'Cost Management'],
    status: 'installed',
    collect: { kind: 'cloud', cloud: 'aws', prefix: 'aws.' },
  },
  {
    id: 'kubernetes',
    name: 'Kubernetes',
    categories: ['Kubernetes', 'Containers', 'Orchestration'],
    status: 'installed',
    logs: 'kubernetes',
    collect: { kind: 'otel', receiver: 'k8s_cluster', fields: ['auth_type: serviceAccount', 'node_conditions_to_report: [Ready, MemoryPressure]'], metrics: K8S },
  },
  {
    id: 'nginx',
    name: 'NGINX',
    categories: ['Web', 'Network'],
    status: 'installed',
    logs: 'file:/var/log/nginx/*.log',
    collect: { kind: 'otel', receiver: 'nginx', fields: ['endpoint: http://localhost:80/nginx_status'], metrics: NGINX },
  },
  {
    id: 'postgresql',
    name: 'PostgreSQL',
    categories: ['Data Stores'],
    status: 'installed',
    logs: 'file:/var/log/postgresql/*.log',
    collect: {
      kind: 'otel',
      receiver: 'postgresql',
      fields: ['endpoint: ${env:PG_ENDPOINT}', 'username: observecraft', 'password: ${env:PG_PASSWORD}', 'tls:\n      insecure: true'],
      metrics: POSTGRES,
    },
  },
  {
    id: 'mongodb',
    name: 'MongoDB',
    categories: ['Data Stores'],
    status: 'installed',
    logs: 'file:/var/log/mongodb/*.log',
    collect: { kind: 'otel', receiver: 'mongodb', fields: ['hosts:\n      - endpoint: ${env:MONGO_ENDPOINT}', 'username: observecraft', 'password: ${env:MONGO_PASSWORD}'], metrics: MONGO },
  },
  {
    id: 'kafka',
    name: 'Kafka',
    logo: 'apachekafka',
    categories: ['Message Queues'],
    status: 'installed',
    collect: { kind: 'otel', receiver: 'kafkametrics', fields: ['brokers: [${env:KAFKA_BROKERS}]', 'protocol_version: 2.0.0', 'scrapers: [brokers, topics, consumers]'], metrics: KAFKA },
  },
  {
    id: 'rabbitmq',
    name: 'RabbitMQ',
    categories: ['Message Queues'],
    status: 'installed',
    collect: { kind: 'otel', receiver: 'rabbitmq', fields: ['endpoint: http://localhost:15672', 'username: ${env:RABBITMQ_USER}', 'password: ${env:RABBITMQ_PASSWORD}'], metrics: RABBIT },
  },
  { id: 'opentelemetry', name: 'OpenTelemetry', categories: ['Tracing', 'Metrics', 'Log Collection'], status: 'installed', collect: { kind: 'otlp', metrics: [] } },
  { id: 'slack', name: 'Slack', categories: ['Collaboration', 'Notifications', 'Alerting'], status: 'installed', collect: { kind: 'notify', settings: NOTIFY_WEBHOOK } },
  {
    id: 'pagerduty',
    name: 'PagerDuty',
    categories: ['Alerting', 'Incidents'],
    status: 'installed',
    collect: { kind: 'notify', settings: [{ key: 'routing_key', label: 'Integration (routing) key', placeholder: 'R0…', secret: true }] },
  },
  {
    id: 'github',
    name: 'GitHub',
    categories: ['Source Control', 'Developer Tools'],
    status: 'installed',
    collect: { kind: 'notify', settings: [{ key: 'app_installation', label: 'GitHub App installation ID', placeholder: '12345678' }] },
  },
  // ---- available
  {
    id: 'azure',
    name: 'Microsoft Azure',
    logo: 'tile:azure',
    categories: ['Azure', 'Cost Management'],
    status: 'installed',
    collect: { kind: 'cloud', cloud: 'azure', prefix: 'azure.' },
  },
  {
    id: 'google-cloud',
    name: 'Google Cloud Platform',
    logo: 'googlecloud',
    categories: ['Google Cloud', 'Cost Management'],
    status: 'installed',
    collect: { kind: 'cloud', cloud: 'gcp', prefix: 'gcp.' },
  },
  {
    id: 'oracle',
    name: 'Oracle Database',
    logo: 'oracle',
    categories: ['Oracle', 'Data Stores'],
    collect: {
      kind: 'otel',
      receiver: 'oracledb',
      fields: ['datasource: oracle://observecraft:${env:ORACLE_PASSWORD}@db-host:1521/ORCL'],
      metrics: [
        ['oracledb.sessions.usage', 'gauge', 'Session usage'],
        ['oracledb.cpu_time', 'count', 'CPU time', 's'],
        ['oracledb.tablespace_size.usage', 'gauge', 'Tablespace usage', 'By'],
      ],
    },
  },
  { id: 'openai', name: 'OpenAI', categories: ['AI/ML'], collect: { kind: 'otlp', metrics: LLM } },
  { id: 'anthropic', name: 'Anthropic', categories: ['AI/ML'], collect: { kind: 'otlp', metrics: LLM } },
  {
    id: 'huggingface',
    name: 'Hugging Face TGI',
    categories: ['AI/ML'],
    collect: {
      kind: 'prometheus',
      target: 'tgi:80/metrics',
      metrics: prom('tgi', [
        ['request_count', 'count', 'Requests'],
        ['request_duration', 'distribution', 'Request duration'],
        ['queue_size', 'gauge', 'Queued requests'],
      ]),
    },
  },
  { id: 'ollama', name: 'Ollama', categories: ['AI/ML'], collect: { kind: 'otlp', metrics: LLM } },
  { id: 'opsgenie', name: 'Opsgenie', categories: ['Alerting', 'Incidents'], collect: { kind: 'notify', settings: [{ key: 'api_key', label: 'API key', placeholder: '••••', secret: true }] } },
  { id: 'microsoft-teams', name: 'Microsoft Teams', logo: 'tile:teams', categories: ['Collaboration', 'Notifications'], collect: { kind: 'notify', settings: NOTIFY_WEBHOOK } },
  {
    id: 'jira',
    name: 'Jira',
    categories: ['Collaboration', 'Incidents'],
    collect: {
      kind: 'notify',
      settings: [
        { key: 'site', label: 'Site URL', placeholder: 'https://your-team.atlassian.net' },
        { key: 'token', label: 'API token', placeholder: '••••', secret: true },
      ],
    },
  },
  {
    id: 'gitlab',
    name: 'GitLab',
    categories: ['Source Control', 'Developer Tools'],
    collect: { kind: 'notify', settings: [{ key: 'token', label: 'Access token', placeholder: 'glpat-…', secret: true }] },
  },
  {
    id: 'terraform',
    name: 'Terraform',
    categories: ['Configuration & Deployment', 'Automation'],
    collect: {
      kind: 'notify',
      settings: [
        { key: 'org', label: 'Organization', placeholder: 'acme' },
        { key: 'token', label: 'API token', placeholder: '••••', secret: true },
      ],
    },
  },
  { id: 'ansible', name: 'Ansible', categories: ['Configuration & Deployment', 'Automation'], collect: { kind: 'notify', settings: NOTIFY_WEBHOOK } },
  {
    id: 'docker',
    name: 'Docker',
    categories: ['Containers'],
    logs: 'docker',
    collect: {
      kind: 'otel',
      receiver: 'docker_stats',
      fields: ['endpoint: unix:///var/run/docker.sock'],
      metrics: [
        ['container.cpu.utilization', 'gauge', 'Container CPU', '1'],
        ['container.memory.usage.total', 'gauge', 'Container memory', 'By'],
        ['container.network.io.usage.rx_bytes', 'count', 'Bytes received', 'By'],
      ],
    },
  },
  {
    id: 'containerd',
    name: 'containerd',
    categories: ['Containers'],
    collect: {
      kind: 'prometheus',
      target: 'localhost:1338',
      metrics: prom('containerd', [
        ['cpu_total', 'count', 'CPU time'],
        ['memory_usage_bytes', 'gauge', 'Memory usage'],
      ]),
    },
  },
  {
    id: 'mysql',
    name: 'MySQL',
    categories: ['Data Stores'],
    logs: 'file:/var/log/mysql/*.log',
    collect: {
      kind: 'otel',
      receiver: 'mysql',
      fields: ['endpoint: ${env:MYSQL_ENDPOINT}', 'username: observecraft', 'password: ${env:MYSQL_PASSWORD}'],
      metrics: [
        ['mysql.connection.count', 'count', 'Connections'],
        ['mysql.operations', 'count', 'Operations'],
        ['mysql.buffer_pool.usage', 'gauge', 'Buffer pool usage', 'By'],
      ],
    },
  },
  {
    id: 'elasticsearch',
    name: 'Elasticsearch',
    categories: ['Data Stores'],
    collect: {
      kind: 'otel',
      receiver: 'elasticsearch',
      fields: ['endpoint: http://localhost:9200'],
      metrics: [
        ['elasticsearch.cluster.health', 'gauge', 'Cluster health'],
        ['elasticsearch.node.operations.completed', 'count', 'Completed operations'],
      ],
    },
  },
  {
    id: 'cassandra',
    name: 'Cassandra',
    logo: 'apachecassandra',
    categories: ['Data Stores'],
    collect: {
      kind: 'otel',
      receiver: 'jmx',
      fields: ['jar_path: /opt/opentelemetry-jmx-metrics.jar', 'endpoint: localhost:7199', 'target_system: cassandra'],
      metrics: [
        ['cassandra.client.request.count', 'count', 'Client requests'],
        ['cassandra.storage.load.count', 'gauge', 'Storage load', 'By'],
      ],
    },
  },
  {
    id: 'memcached',
    name: 'Memcached',
    logo: 'tile:memcached',
    categories: ['Caching', 'Data Stores'],
    collect: {
      kind: 'otel',
      receiver: 'memcached',
      fields: ['endpoint: localhost:11211'],
      metrics: [
        ['memcached.operations', 'count', 'Operations'],
        ['memcached.current_items', 'gauge', 'Items stored'],
      ],
    },
  },
  {
    id: 'haproxy',
    name: 'HAProxy',
    logo: 'tile:haproxy',
    categories: ['Network', 'Web'],
    collect: {
      kind: 'otel',
      receiver: 'haproxy',
      fields: ['endpoint: file:///var/run/haproxy.ipc'],
      metrics: [
        ['haproxy.requests.total', 'count', 'HTTP requests'],
        ['haproxy.connections.total', 'count', 'Connections'],
      ],
    },
  },
  {
    id: 'istio',
    name: 'Istio',
    categories: ['Kubernetes', 'Network'],
    collect: {
      kind: 'prometheus',
      target: 'istiod.istio-system:15014',
      metrics: prom('istio', [
        ['requests_total', 'count', 'Requests'],
        ['request_duration_milliseconds', 'distribution', 'Request duration'],
      ]),
    },
  },
  {
    id: 'linux',
    name: 'Linux Host',
    categories: ['OS & System'],
    logs: 'journald',
    os: ['Linux'],
    collect: { kind: 'otel', receiver: 'hostmetrics', fields: ['scrapers:\n      cpu: {}\n      memory: {}\n      filesystem: {}\n      network: {}'], metrics: HOST },
  },
  {
    id: 'vault',
    name: 'HashiCorp Vault',
    logo: 'vault',
    categories: ['Security'],
    collect: {
      kind: 'prometheus',
      target: 'vault:8200/v1/sys/metrics?format=prometheus',
      metrics: prom('vault', [
        ['core_unsealed', 'gauge', 'Unsealed state'],
        ['token_count', 'gauge', 'Active tokens'],
      ]),
    },
  },
  { id: 'cloudflare', name: 'Cloudflare', categories: ['Network', 'Security'], collect: { kind: 'notify', settings: [{ key: 'token', label: 'API token', placeholder: '••••', secret: true }] } },
  {
    id: 'jenkins',
    name: 'Jenkins',
    categories: ['Developer Tools', 'Automation'],
    collect: {
      kind: 'prometheus',
      target: 'jenkins:8080/prometheus',
      metrics: prom('jenkins', [
        ['builds_duration_milliseconds_summary', 'distribution', 'Build duration'],
        ['executor_in_use_value', 'gauge', 'Executors in use'],
      ]),
    },
  },
  {
    id: 'github-actions',
    name: 'GitHub Actions',
    logo: 'githubactions',
    categories: ['Developer Tools', 'Testing'],
    collect: { kind: 'notify', settings: [{ key: 'app_installation', label: 'GitHub App installation ID', placeholder: '12345678' }] },
  },
  { id: 'sentry', name: 'Sentry', categories: ['Developer Tools'], collect: { kind: 'notify', settings: NOTIFY_WEBHOOK } },
  {
    id: 'helm',
    name: 'Helm',
    categories: ['Kubernetes', 'Configuration & Deployment'],
    collect: {
      kind: 'otel',
      receiver: 'k8sobjects',
      fields: ['objects:\n      - name: secrets\n        label_selector: owner=helm'],
      metrics: [['helm.release.status', 'gauge', 'Release status by chart']],
    },
  },
  { id: 'prometheus', name: 'Prometheus', categories: ['Metrics'], collect: { kind: 'prometheus', target: 'my-app:9090', metrics: [] } },
  { id: 'nodejs', name: 'Node.js', logo: 'nodedotjs', categories: ['Languages', 'Tracing'], collect: { kind: 'otlp', metrics: RUNTIME('nodejs') } },
  { id: 'python', name: 'Python', categories: ['Languages', 'Tracing'], collect: { kind: 'otlp', metrics: RUNTIME('python') } },
  { id: 'go', name: 'Go', categories: ['Languages', 'Tracing'], collect: { kind: 'otlp', metrics: RUNTIME('go') } },
  { id: 'java', name: 'Java', logo: 'openjdk', categories: ['Languages', 'Tracing'], collect: { kind: 'otlp', metrics: RUNTIME('jvm') } },
  {
    id: 'consul',
    name: 'Consul',
    categories: ['Network', 'Configuration & Deployment'],
    collect: {
      kind: 'prometheus',
      target: 'consul:8500/v1/agent/metrics?format=prometheus',
      metrics: prom('consul', [
        ['raft_leader', 'gauge', 'Raft leader'],
        ['catalog_services', 'gauge', 'Registered services'],
      ]),
    },
  },
  {
    id: 'etcd',
    name: 'etcd',
    categories: ['Kubernetes', 'Data Stores'],
    collect: {
      kind: 'prometheus',
      target: 'etcd:2379',
      metrics: prom('etcd', [
        ['server_has_leader', 'gauge', 'Has leader'],
        ['disk_wal_fsync_duration_seconds', 'distribution', 'WAL fsync duration'],
      ]),
    },
  },
  {
    id: 'fluent-bit',
    name: 'Fluent Bit',
    logo: 'fluentbit',
    categories: ['Log Collection'],
    collect: {
      kind: 'prometheus',
      target: 'localhost:2020/api/v1/metrics/prometheus',
      metrics: prom('fluentbit', [
        ['input_records_total', 'count', 'Input records'],
        ['output_errors_total', 'count', 'Output errors'],
      ]),
    },
  },
  {
    id: 'snowflake',
    name: 'Snowflake',
    categories: ['Data Stores', 'Cost Management'],
    collect: {
      kind: 'otel',
      receiver: 'snowflake',
      fields: ['account: ${env:SNOWFLAKE_ACCOUNT}', 'username: observecraft', 'password: ${env:SNOWFLAKE_PASSWORD}', 'warehouse: OBSERVABILITY_WH'],
      metrics: [
        ['snowflake.query.executed', 'count', 'Queries executed'],
        ['snowflake.billing.warehouse.total_credit.total', 'gauge', 'Credits used'],
      ],
    },
  },
  {
    id: 'databricks',
    name: 'Databricks',
    categories: ['Data Stores', 'AI/ML'],
    collect: {
      kind: 'notify',
      settings: [
        { key: 'host', label: 'Workspace URL', placeholder: 'https://adb-….azuredatabricks.net' },
        { key: 'token', label: 'Access token', placeholder: '••••', secret: true },
      ],
    },
  },
  {
    id: 'authentik',
    name: 'authentik',
    categories: ['Security', 'Log Collection'],
    about: 'Events of an authentik identity provider (logins, access requests, users and policies changed, errors), read through its API and stored as logs.',
    collect: {
      kind: 'notify',
      settings: [
        { key: 'base_url', label: 'authentik URL', placeholder: 'https://auth.example.com' },
        { key: 'token', label: 'API token', placeholder: '••••', secret: true },
      ],
    },
  },
  {
    id: 'netbird',
    name: 'NetBird',
    categories: ['Network', 'Security', 'Log Collection'],
    about:
      'Peers of a NetBird overlay network (connected, last seen, expired logins, OS, client versions) as metrics and its audit log (peers, policies, routes, setup keys, users) as logs, read through its management API, with a dashboard.',
    collect: {
      kind: 'notify',
      settings: [
        { key: 'base_url', label: 'Management URL', placeholder: 'https://api.netbird.io' },
        { key: 'token', label: 'Personal access token', placeholder: '••••', secret: true },
      ],
    },
  },
  {
    id: 'fleetdm',
    name: 'FleetDM',
    categories: ['Security', 'OS & System', 'Log Collection'],
    about:
      'Hosts managed by a Fleet server (online, last seen, OS versions, disk, failing policies, vulnerabilities, MDM enrollment) and its policies as metrics, and its activity feed (logins, policies and queries, scripts, software, hosts locked or wiped) as logs, read through its API, with a dashboard.',
    collect: {
      kind: 'notify',
      settings: [
        { key: 'base_url', label: 'Fleet URL', placeholder: 'https://fleet.example.com' },
        { key: 'token', label: 'API token', placeholder: '••••', secret: true },
      ],
    },
  },
  {
    id: 'okta',
    name: 'Okta',
    categories: ['Security'],
    collect: {
      kind: 'notify',
      settings: [
        { key: 'domain', label: 'Okta domain', placeholder: 'acme.okta.com' },
        { key: 'token', label: 'API token', placeholder: '••••', secret: true },
      ],
    },
  },
  {
    id: 'palo-alto',
    name: 'Palo Alto Networks Firewall',
    logo: 'paloaltonetworks',
    categories: ['Security', 'Network'],
    logs: 'syslog',
    collect: { kind: 'notify', settings: [{ key: 'syslog_port', label: 'Syslog port', placeholder: '514' }] },
  },
  {
    id: 'cypress',
    name: 'Cypress',
    categories: ['Testing'],
    collect: {
      kind: 'otlp',
      metrics: [
        ['test.run.duration', 'distribution', 'Run duration', 's'],
        ['test.failures', 'count', 'Failed tests'],
      ],
    },
  },
  {
    id: 'selenium',
    name: 'Selenium Grid',
    logo: 'selenium',
    categories: ['Testing'],
    collect: {
      kind: 'prometheus',
      target: 'selenium-hub:4444',
      metrics: prom('selenium_grid', [
        ['sessions_queued', 'gauge', 'Queued sessions'],
        ['node_count', 'gauge', 'Nodes'],
      ]),
    },
  },
  {
    id: 'airflow',
    name: 'Airflow',
    logo: 'apacheairflow',
    categories: ['Automation', 'Orchestration'],
    collect: {
      kind: 'otel',
      receiver: 'statsd',
      fields: ['endpoint: localhost:8125'],
      metrics: [
        ['airflow.dagrun.duration.success', 'distribution', 'Successful DAG run duration', 'ms'],
        ['airflow.ti_failures', 'count', 'Task instance failures'],
      ],
    },
  },
  { id: 'circleci', name: 'CircleCI', categories: ['Developer Tools', 'Testing'], collect: { kind: 'notify', settings: [{ key: 'token', label: 'API token', placeholder: '••••', secret: true }] } },
  {
    id: 'zendesk',
    name: 'Zendesk',
    categories: ['Collaboration'],
    collect: {
      kind: 'notify',
      settings: [
        { key: 'subdomain', label: 'Subdomain', placeholder: 'acme' },
        { key: 'token', label: 'API token', placeholder: '••••', secret: true },
      ],
    },
  },
  {
    id: 'twilio',
    name: 'Twilio',
    categories: ['Alerting', 'Notifications'],
    collect: {
      kind: 'notify',
      settings: [
        { key: 'sid', label: 'Account SID', placeholder: 'AC…' },
        { key: 'token', label: 'Auth token', placeholder: '••••', secret: true },
      ],
    },
  },
  { id: 'stripe', name: 'Stripe', categories: ['Developer Tools'], collect: { kind: 'notify', settings: [{ key: 'key', label: 'Restricted API key', placeholder: 'rk_live_…', secret: true }] } },
];

type CloudId = 'aws' | 'azure' | 'gcp';

/** The cloud integrations: their page, where their metrics come from, how a service tile is set up. */
const CLOUDS: Record<CloudId, { page: string; source: string; steps: string[] }> = {
  aws: {
    page: '/integrations/aws',
    source: 'CloudWatch, through the Amazon Web Services integration',
    steps: [
      'Add your AWS account on the Amazon Web Services integration page (CloudFormation, Terraform or manually).',
      'Keep this service’s namespace among the account’s collected namespaces (Metric Collection).',
    ],
  },
  gcp: {
    page: '/integrations/gcp',
    source: 'Cloud Monitoring, through the Google Cloud integration',
    steps: [
      'Add a service account on the Google Cloud integration page (Cloud Shell or Terraform); ObserveCraft impersonates it.',
      'Keep this service enabled for the service account (Metric Collection).',
    ],
  },
  azure: {
    page: '/integrations/azure',
    source: 'Azure Monitor, through the Microsoft Azure integration',
    steps: [
      'Add an app registration on the Microsoft Azure integration page (Terraform or an existing app registration).',
      'Keep this service’s resource provider enabled for the app registration (Metric Collection).',
    ],
  },
};

/** AWS services the demo account collects (mock mode). */
const DEMO_AWS = new Set(['AWS/ApiGateway', 'AWS/ApplicationELB', 'AWS/CloudFront', 'AWS/DynamoDB', 'AWS/EC2', 'AWS/ECS', 'ECS/ContainerInsights', 'AWS/Lambda', 'AWS/RDS', 'AWS/S3', 'AWS/WAFV2']);

/** One tile per CloudWatch namespace of the AWS integration's catalog (server/seeds/aws_integration.json). */
const AWS_SERVICES: Entry[] = awsCatalog.namespaces.map((n) => ({
  id: n.id,
  name: n.name,
  logo: `tile:${n.id}`,
  categories: n.categories,
  status: DEMO_AWS.has(n.namespace) ? 'installed' : undefined,
  collect: { kind: 'cloud', cloud: 'aws', service: n.namespace, prefix: awsMetricPrefix(n.namespace) },
}));

/** Google Cloud services the demo service account collects (mock mode). */
const DEMO_GCP = new Set(['gce', 'cloudsql', 'gcs', 'run']);

/** One tile per service of the GCP integration's catalog (server/seeds/gcp_integration.json). */
const GCP_SERVICES: Entry[] = gcpCatalog.services.map((s) => ({
  id: s.id,
  name: s.name,
  logo: `tile:${s.id}`,
  categories: s.categories,
  status: DEMO_GCP.has(s.short) ? 'installed' : undefined,
  collect: { kind: 'cloud', cloud: 'gcp', service: s.prefix.replace(/\/$/, ''), prefix: `gcp.${s.short}.` },
}));

/** Azure services the demo app registration collects (mock mode). */
const DEMO_AZURE = new Set(['Microsoft.Compute/virtualMachines', 'Microsoft.Web/sites', 'Microsoft.Sql/servers/databases', 'Microsoft.Storage/storageAccounts']);

/** One tile per resource type of the Azure integration's catalog (server/seeds/azure_integration.json). */
const AZURE_SERVICES: Entry[] = azureCatalog.resource_types.map((t) => ({
  id: t.id,
  name: t.name,
  logo: `tile:${t.id}`,
  categories: t.categories,
  status: DEMO_AZURE.has(t.type) ? 'installed' : undefined,
  collect: { kind: 'cloud', cloud: 'azure', service: t.type, prefix: `${t.prefix}.` },
}));

const CORE_LINKS = linkCore(CURATED, coreCatalog.integrations);
const CATALOG: Entry[] = [
  ...CURATED.map((e) => ({ ...e, core: CORE_LINKS.linked.get(e.id) })),
  ...AWS_SERVICES,
  ...AZURE_SERVICES,
  ...GCP_SERVICES,
  ...CORE_LINKS.unlinked.map((c): Entry => ({ id: c.id, name: c.name, categories: c.categories, os: c.supportedOs, collect: { kind: 'agent' }, core: c })),
];

// ---------------------------------------------------------------- derivation

const COLLECTOR_VERSION = '0.112.0';
const HOSTS = ['i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003', 'i-0a1b2c3d4e5f60006', 'ip-10-42-3-17', 'ip-10-42-7-201'];

/** Session-local install state (the mock's "database"). */
const statusOverride = new Map<string, IntegrationStatus>();

function statusOf(e: Entry): IntegrationStatus {
  return statusOverride.get(e.id) ?? e.status ?? 'available';
}

/** Installed in this session while nothing sent its data yet, as after Install before its Agent check is configured. */
function waitingForData(e: Entry): boolean {
  return statusOverride.get(e.id) === 'installed' && !e.status && e.collect.kind !== 'notify' && e.collect.kind !== 'cloud';
}

function summary(e: Entry): IntegrationSummary {
  const status = statusOf(e);
  return {
    id: e.id,
    name: e.name,
    publisher: e.core?.publisher ?? 'ObserveCraft',
    logo: e.logo ?? e.id,
    status,
    categories: e.categories,
    description: e.core?.description || `Monitor the health and performance of ${e.name}.`,
    warning: status === 'warning' ? e.warning : undefined,
    repository: e.core?.repository,
    waitingForData: waitingForData(e) || undefined,
  };
}

function otelFragment(id: string, receiverKey: string, body: string, signal = 'metrics'): string {
  return [
    'receivers:',
    `  ${receiverKey}:`,
    body,
    '',
    'service:',
    '  pipelines:',
    `    ${signal}/${id}:`,
    `      receivers: [${receiverKey}]`,
    '      processors: [resourcedetection, batch]',
    '      exporters: [otlp/observecraft]',
  ].join('\n');
}

function configsFor(e: Entry): CollectorConfig[] {
  const out: CollectorConfig[] = [];
  const c = e.collect;
  const file = (name: string) => `conf.d/${e.id}${name}.yaml`;
  switch (c.kind) {
    case 'otel':
      out.push({
        target: 'otel-collector',
        title: `OpenTelemetry Collector — ${c.receiver} receiver`,
        filename: file(''),
        language: 'yaml',
        content: otelFragment(e.id, c.receiver, [...c.fields.map((f) => `    ${f}`), '    collection_interval: 15s'].join('\n')),
      });
      break;
    case 'prometheus':
      out.push({
        target: 'otel-collector',
        title: 'OpenTelemetry Collector — Prometheus scrape',
        filename: file(''),
        language: 'yaml',
        content: otelFragment(
          e.id,
          `prometheus/${e.id}`,
          ['    config:', '      scrape_configs:', `        - job_name: ${e.id}`, '          scrape_interval: 30s', '          static_configs:', `            - targets: ['${c.target}']`].join('\n'),
        ),
      });
      break;
    case 'otlp':
      out.push({
        target: 'otel-collector',
        title: 'OpenTelemetry Collector — OTLP intake (already enabled by default)',
        filename: 'conf.d/otlp.yaml',
        language: 'yaml',
        content: otelFragment(e.id, 'otlp', ['    protocols:', '      grpc:', '        endpoint: 0.0.0.0:4317', '      http:', '        endpoint: 0.0.0.0:4318'].join('\n')),
      });
      break;
    case 'notify':
    case 'agent':
    case 'cloud':
      break;
  }

  if (e.logs) {
    const [kind, glob] = e.logs.split(':');
    const source =
      kind === 'file'
        ? ['    type: file', `    include: ["${glob}"]`]
        : kind === 'kubernetes'
          ? ['    type: kubernetes_logs']
          : kind === 'journald'
            ? ['    type: journald']
            : kind === 'docker'
              ? ['    type: docker_logs']
              : kind === 'syslog'
                ? ['    type: syslog', '    mode: udp', '    address: 0.0.0.0:514']
                : ['    type: file', '    include: ["C:\\\\ProgramData\\\\ObserveCraft\\\\eventlog\\\\*.json"]'];
    out.push({
      target: 'vector',
      title: 'Vector — log collection',
      filename: `vector.d/${e.id}.yaml`,
      language: 'yaml',
      content: [
        'sources:',
        `  ${e.id.replace(/-/g, '_')}_logs:`,
        ...source,
        '',
        'transforms:',
        `  ${e.id.replace(/-/g, '_')}_tag:`,
        '    type: remap',
        `    inputs: [${e.id.replace(/-/g, '_')}_logs]`,
        '    source: |',
        `      .source = "${e.id}"`,
        `      .service = .service ?? "${e.id}"`,
        ...(e.id === 'apache' || e.id === 'nginx' ? [`      . |= parse_${e.id === 'apache' ? 'apache_log' : 'nginx_log'}(.message, "combined") ?? {}`] : []),
        '',
        '# Routed to the ObserveCraft logs pipeline by the aggregator.',
      ].join('\n'),
    });
  }
  return out;
}

function metricsFor(e: Entry): CollectedMetric[] {
  const c = e.collect;
  if (c.kind === 'notify' || c.kind === 'agent' || c.kind === 'cloud') return [];
  return c.metrics.map(([name, type, description, unit]) => ({ name, type, description, unit }));
}

function hostsFor(e: Entry, status: IntegrationStatus): IntegrationHost[] {
  if (status === 'available' || waitingForData(e) || e.collect.kind === 'notify' || e.collect.kind === 'cloud') return [];
  const r = rng(e.id.length * 131 + e.id.charCodeAt(0));
  const n = 1 + Math.floor(r() * 4);
  const now = Date.now();
  return HOSTS.slice(0, n).map((host, i) => ({
    host,
    status: status === 'detected' ? 'detected' : status === 'warning' && i === 0 ? 'warning' : 'ok',
    collectorVersion: COLLECTOR_VERSION,
    lastReported: now - Math.round(r() * 90e3),
    message: status === 'warning' && i === 0 ? e.warning : status === 'detected' ? `${e.name} process detected; integration not configured` : undefined,
  }));
}

function detail(e: Entry): IntegrationDetail {
  const s = summary(e);
  const c = e.collect;
  const dataTypes =
    c.kind === 'agent'
      ? coreDataTypes(e.core!)
      : [
          ...(c.kind !== 'notify' ? ['Metrics'] : []),
          ...(e.logs ? ['Logs'] : []),
          ...(c.kind === 'otlp' && e.categories.includes('Tracing') ? ['Traces'] : []),
          ...(c.kind === 'notify' ? ['Events'] : []),
        ];
  const how =
    c.kind === 'otel'
      ? `the OpenTelemetry Collector's ${c.receiver} receiver`
      : c.kind === 'prometheus'
        ? `a Prometheus scrape of ${c.target}`
        : c.kind === 'cloud'
          ? `${CLOUDS[c.cloud].source}${c.service ? ` (${c.service})` : ''}`
          : c.kind === 'otlp'
            ? 'OTLP from the OpenTelemetry SDK in your application'
            : c.kind === 'agent'
              ? `the Datadog Agent's ${e.core!.check} check`
              : 'the provider API — no collector changes are required';
  const steps =
    c.kind === 'notify'
      ? [`Fill in the ${e.name} settings below.`, 'Click Install. ObserveCraft validates the credentials.', `Reference @${e.id} in monitor notifications.`]
      : c.kind === 'otlp'
        ? [
            'Add the OpenTelemetry SDK and auto-instrumentation to your service.',
            'Point OTEL_EXPORTER_OTLP_ENDPOINT at the local collector (port 4317).',
            'Click Install to enable the bundled dashboards and monitors.',
          ]
        : c.kind === 'agent'
          ? AGENT_STEPS(e.core!)
          : c.kind === 'cloud'
            ? CLOUDS[c.cloud].steps
            : [
                `Click Install. The fleet manager adds the configuration below to collectors on matching hosts.`,
                'Set any referenced environment variables (credentials, endpoints) via your secret store.',
                `Data appears within a few minutes under the "${e.id}" source.`,
              ];
  return {
    ...s,
    overview: `${e.about ? `${e.about} ` : ''}This integration collects ${e.name} telemetry through ${how}.`,
    offerings: e.core?.offerings.length ? e.core.offerings : ['Integration'],
    supportedOs: e.os ?? (e.core?.supportedOs.length ? e.core.supportedOs : ['Linux', 'Windows', 'macOS']),
    dataTypes,
    minCollectorVersion: COLLECTOR_VERSION,
    setupSteps: steps,
    configureUrl: c.kind === 'cloud' ? CLOUDS[c.cloud].page : undefined,
    configs: configsFor(e),
    settings: c.kind === 'notify' ? c.settings : [],
    metrics: metricsFor(e),
    logs: e.logs
      ? `Logs are collected by Vector and tagged source:${e.id}.`
      : e.core?.logSource
        ? `The Datadog Agent collects logs with source:${e.core.logSource} when log collection is enabled for the check.`
        : null,
    agent: e.core ? { check: e.core.check, configurable: e.core.configurable, source: coreCatalog.source } : undefined,
    coreId: e.core?.id,
    hosts: hostsFor(e, s.status),
    resources:
      c.kind === 'notify'
        ? []
        : c.kind === 'agent'
          ? e.core!.monitors.map((name) => ({ kind: 'monitor' as const, name, description: 'Recommended monitor from integrations-core.' }))
          : [
              { kind: 'dashboard', name: `${e.name} - Overview`, description: 'Throughput, errors and saturation at a glance.' },
              { kind: 'monitor', name: `${e.name} is not reporting`, description: 'No data from the integration for 10 minutes.' },
              { kind: 'monitor', name: `${e.name} error rate is high`, description: 'Error rate above 5% for 5 minutes.' },
            ],
    releaseNotes:
      c.kind === 'agent'
        ? []
        : [
            { version: '1.3.0', date: '2026-09-02', notes: ['Added default monitors.', `Updated for collector ${COLLECTOR_VERSION}.`] },
            { version: '1.2.1', date: '2026-06-18', notes: ['Fixed unit metadata on several metrics.'] },
            { version: '1.0.0', date: '2026-01-10', notes: ['Initial release.'] },
          ],
  };
}

function coreDataTypes(c: CoreIntegration): string[] {
  if (c.dataTypes.length) return c.dataTypes;
  return [...(c.metricPrefix ? ['Metrics'] : []), ...(c.logSource ? ['Logs'] : [])];
}

const AGENT_STEPS = (c: CoreIntegration) => [
  'Install the Datadog Agent on the host and point it at ObserveCraft (DD_SITE / DD_DD_URL and your ingest key as DD_API_KEY).',
  c.configurable
    ? `Fill in the settings below and save the result as conf.d/${c.check}.d/conf.yaml in the Agent's configuration directory.`
    : `Configure the ${c.name} integration as its integrations-core documentation describes.`,
  'Restart the Agent. Data appears within a few minutes; detection uses metrics received during the last 24 hours.',
];

const BY_ID = new Map(CATALOG.map((e) => [e.id, e]));

function find(id: string): Entry {
  const e = BY_ID.get(id);
  if (!e) throw new Error(`Integration ${id} not found`);
  return e;
}

// ---------------------------------------------------------------- endpoints

/** The whole catalog; searches filter it in the browser (`filterCatalog`). */
export function listCatalog(): Promise<IntegrationSummary[]> {
  return delay(CATALOG.map(summary));
}

export function getIntegration(id: string): Promise<IntegrationDetail> {
  return delay(detail(find(id)));
}

export function installIntegration(id: string): Promise<IntegrationDetail> {
  statusOverride.set(id, 'installed');
  return delay(detail(find(id)), 400);
}

export function uninstallIntegration(id: string): Promise<IntegrationDetail> {
  const e = find(id);
  statusOverride.set(id, e.status === 'detected' ? 'detected' : 'available');
  return delay(detail(e), 300);
}

/** Static catalog metadata, without the demo installation state or generated hosts. */
export function catalogMetadata(): IntegrationSummary[] {
  return CATALOG.map((e) => ({ ...summary(e), status: 'available', warning: undefined, waitingForData: undefined }));
}

/** The metrics an integration declares (OTel and Prometheus templates), without building its whole detail. */
export function declaredMetrics(id: string): { name: string }[] {
  return metricsFor(find(id));
}

export function integrationMetadata(id: string): IntegrationDetail {
  const e = find(id);
  return {
    ...detail(e),
    status: 'available',
    warning: undefined,
    waitingForData: undefined,
    hosts: [],
    resources: [],
    releaseNotes: [],
    setupSteps:
      e.collect.kind === 'agent'
        ? AGENT_STEPS(e.core!)
        : e.collect.kind === 'cloud'
          ? CLOUDS[e.collect.cloud].steps
          : e.collect.kind === 'notify'
            ? ['Configure a supported notification channel in Organization Settings. This catalog does not provision provider connections.']
            : [
                'Apply the configuration template to your own collector and supply the required endpoints and credentials.',
                'Add the receiver to the appropriate collector service pipeline and configure an exporter to the ObserveCraft intake with your organization ingest key.',
                'Restart or reload the collector. Detection is based on matching metrics received during the last 24 hours.',
              ],
  };
}

/** Metric prefixes that identify the integration in received telemetry, from integrations-core. */
export function catalogPrefixes(id: string): string[] {
  const e = find(id);
  if (e.collect.kind === 'cloud') return [e.collect.prefix];
  const prefix = e.core?.detectionPrefix;
  return prefix ? [prefix] : [];
}
