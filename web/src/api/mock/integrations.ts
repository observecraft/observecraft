// In-browser stand-in for the Integrations endpoints (see api/integrations/client.ts).
// Each catalog entry declares HOW it is collected; config files, metric lists and hosts are
// derived from that, so the Configure tab shows the collector config "Install" would apply.
// Generated configs are templates — verify field names against the pinned collector version.
import type {
  CatalogRequest,
  CatalogResponse,
  CollectedMetric,
  CollectorConfig,
  IntegrationDetail,
  IntegrationHost,
  IntegrationStatus,
  IntegrationSummary,
  MarketplaceApp,
  MarketplaceRequest,
} from '../integrations/types';
import { rng } from './generator';

const delay = <T,>(value: T, ms = 100 + Math.random() * 150) => new Promise<T>((r) => setTimeout(() => r(value), ms));

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
  return delay({ featured: FEATURED, all: [...FEATURED, ...MORE].sort() }, 30);
}

// ---------------------------------------------------------------- collection specs

type M = [name: string, type: CollectedMetric['type'], description: string, unit?: string];

type Collect =
  | { kind: 'otel'; receiver: string; fields: string[]; metrics: M[] }
  | { kind: 'prometheus'; target: string; metrics: M[] }
  | { kind: 'cloudwatch'; namespace: string; metrics: string[] }
  | { kind: 'otlp'; metrics: M[] }
  | { kind: 'notify'; settings: IntegrationDetail['settings'] };

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

const CATALOG: Entry[] = [
  // ---- autodetected
  { id: 'apache', name: 'Apache', categories: ['Web', 'Log Collection', 'Metrics'], status: 'detected', logs: 'file:/var/log/apache2/*.log', collect: { kind: 'otel', receiver: 'apache', fields: ['endpoint: http://localhost:80/server-status?auto'], metrics: APACHE } },
  { id: 'argo-rollouts', name: 'Argo Rollouts', logo: 'argo', categories: ['Kubernetes', 'Configuration & Deployment'], status: 'detected', collect: { kind: 'prometheus', target: 'argo-rollouts-metrics.argo-rollouts:8090', metrics: prom('rollout', [['info', 'gauge', 'Rollout info and phase'], ['reconcile_count', 'count', 'Reconciliations'], ['events_total', 'count', 'Rollout events']]) } },
  { id: 'celery', name: 'Celery', categories: ['Message Queues', 'Languages'], status: 'detected', collect: { kind: 'prometheus', target: 'celery-flower:5555', metrics: prom('flower', [['events_total', 'count', 'Task events by type'], ['worker_online', 'gauge', 'Online workers'], ['task_runtime_seconds', 'distribution', 'Task runtime']]) } },
  { id: 'envoy', name: 'Envoy', logo: 'envoyproxy', categories: ['Network', 'Web'], status: 'detected', collect: { kind: 'prometheus', target: 'localhost:9901/stats/prometheus', metrics: prom('envoy', [['cluster_upstream_rq_total', 'count', 'Upstream requests'], ['cluster_upstream_cx_active', 'gauge', 'Active upstream connections'], ['http_downstream_rq_xx', 'count', 'Downstream responses by class']]) } },
  { id: 'gunicorn', name: 'Gunicorn', categories: ['Web', 'Languages'], status: 'detected', logs: 'file:/var/log/gunicorn/*.log', collect: { kind: 'otel', receiver: 'statsd', fields: ['endpoint: localhost:8125', 'aggregation_interval: 15s'], metrics: [['gunicorn.requests', 'rate', 'Requests per second'], ['gunicorn.request.duration', 'distribution', 'Request duration', 'ms'], ['gunicorn.workers', 'gauge', 'Worker processes']] } },
  { id: 'keda', name: 'KEDA', categories: ['Kubernetes', 'Log Collection', 'Metrics', 'Security'], status: 'detected', about: 'KEDA is a Kubernetes-based event-driven autoscaler.', collect: { kind: 'prometheus', target: 'keda-operator-metrics-apiserver.keda:8080', metrics: prom('keda', [['scaler_active', 'gauge', 'Active scalers'], ['scaler_metrics_value', 'gauge', 'Current metric value per scaler'], ['scaled_object_errors_total', 'count', 'ScaledObject errors']]) } },
  { id: 'cluster-autoscaler', name: 'Kubernetes Cluster Autoscaler', logo: 'kubernetes', categories: ['Kubernetes', 'Orchestration'], status: 'detected', collect: { kind: 'prometheus', target: 'cluster-autoscaler.kube-system:8085', metrics: prom('cluster_autoscaler', [['nodes_count', 'gauge', 'Nodes by state'], ['unschedulable_pods_count', 'gauge', 'Unschedulable pods'], ['scaled_up_nodes_total', 'count', 'Scale-up events']]) } },
  { id: 'supervisord', name: 'Supervisord', categories: ['OS & System'], status: 'detected', collect: { kind: 'prometheus', target: 'localhost:9876', metrics: prom('supervisord', [['up', 'gauge', 'Process is running'], ['start_time_seconds', 'gauge', 'Process start time'], ['exit_status', 'gauge', 'Last exit status']]) } },
  { id: 'tomcat', name: 'Tomcat', logo: 'apachetomcat', categories: ['Web', 'Languages'], status: 'detected', logs: 'file:/opt/tomcat/logs/*.log', collect: { kind: 'otel', receiver: 'jmx', fields: ['jar_path: /opt/opentelemetry-jmx-metrics.jar', 'endpoint: localhost:9010', 'target_system: tomcat'], metrics: [['tomcat.sessions', 'gauge', 'Active sessions'], ['tomcat.errors', 'count', 'Request errors'], ['tomcat.request_count', 'count', 'Requests'], ['tomcat.threads', 'gauge', 'Threads by state'], ['tomcat.processing_time', 'count', 'Processing time', 'ms']] } },
  { id: 'velero', name: 'Velero', categories: ['Kubernetes', 'Data Stores'], status: 'detected', collect: { kind: 'prometheus', target: 'velero.velero:8085', metrics: prom('velero', [['backup_success_total', 'count', 'Successful backups'], ['backup_failure_total', 'count', 'Failed backups'], ['backup_last_successful_timestamp', 'gauge', 'Last successful backup']]) } },
  { id: 'windows-cert-store', name: 'Windows Certificate Store', categories: ['Windows', 'Security', 'OS & System'], status: 'detected', os: ['Windows'], collect: { kind: 'prometheus', target: 'localhost:9182', metrics: prom('windows_certstore', [['cert_expiry_seconds', 'gauge', 'Seconds until certificate expiry'], ['cert_count', 'gauge', 'Certificates per store']]) } },
  // ---- installed
  { id: 'coredns', name: 'CoreDNS', categories: ['Kubernetes', 'Network'], status: 'installed', collect: { kind: 'prometheus', target: 'kube-dns.kube-system:9153', metrics: prom('coredns', [['dns_requests_total', 'count', 'DNS requests'], ['dns_responses_total', 'count', 'Responses by rcode'], ['dns_request_duration_seconds', 'distribution', 'Request duration']]) } },
  { id: 'redis', name: 'Redis', categories: ['Data Stores', 'Caching'], status: 'installed', logs: 'file:/var/log/redis/*.log', collect: { kind: 'otel', receiver: 'redis', fields: ['endpoint: ${env:REDIS_ENDPOINT}', 'password: ${env:REDIS_PASSWORD}'], metrics: REDIS } },
  { id: 'dotnet', name: '.NET Runtime Metrics', categories: ['Languages', 'Tracing'], status: 'installed', collect: { kind: 'otlp', metrics: RUNTIME('dotnet') } },
  { id: 'active-directory', name: 'Active Directory', categories: ['Windows', 'Security'], status: 'installed', os: ['Windows'], logs: 'windows_event_log', collect: { kind: 'otel', receiver: 'active_directory_ds', fields: [], metrics: [['active_directory.ds.replication.network.io', 'count', 'Replication traffic', 'By'], ['active_directory.ds.thread.count', 'gauge', 'Threads in use'], ['active_directory.ds.bind.rate', 'rate', 'Binds per second'], ['active_directory.ds.ldap.search.rate', 'rate', 'LDAP searches per second']] } },
  { id: 'aws', name: 'Amazon Web Services', logo: 'amazonwebservices', categories: ['AWS', 'Cost Management'], status: 'installed', collect: { kind: 'cloudwatch', namespace: 'AWS/Usage', metrics: ['CallCount', 'ResourceCount'] } },
  { id: 'aws-api-gateway', name: 'Amazon API Gateway Integration', logo: 'amazonapigateway', categories: ['AWS', 'Web'], status: 'installed', collect: { kind: 'cloudwatch', namespace: 'AWS/ApiGateway', metrics: ['Count', 'Latency', '4XXError', '5XXError'] } },
  { id: 'aws-appstream', name: 'Amazon AppStream', logo: 'tile:aws-appstream', categories: ['AWS'], status: 'warning', warning: 'Missing IAM permission appstream:DescribeFleets on role observecraft-integration.', collect: { kind: 'cloudwatch', namespace: 'AWS/AppStream', metrics: ['ActualCapacity', 'InUseCapacity', 'CapacityUtilization'] } },
  { id: 'aws-athena', name: 'Amazon Athena', logo: 'tile:aws-athena', categories: ['AWS', 'Data Stores'], status: 'warning', warning: 'No metrics received in the last 24h. Enable CloudWatch metrics on the Athena workgroup.', collect: { kind: 'cloudwatch', namespace: 'AWS/Athena', metrics: ['TotalExecutionTime', 'ProcessedBytes', 'QueryQueueTime'] } },
  { id: 'aws-bedrock', name: 'Amazon Bedrock', logo: 'tile:aws-bedrock', categories: ['AWS', 'AI/ML'], status: 'installed', collect: { kind: 'cloudwatch', namespace: 'AWS/Bedrock', metrics: ['Invocations', 'InvocationLatency', 'InputTokenCount', 'OutputTokenCount'] } },
  { id: 'aws-lambda', name: 'AWS Lambda', logo: 'awslambda', categories: ['AWS'], status: 'installed', collect: { kind: 'cloudwatch', namespace: 'AWS/Lambda', metrics: ['Invocations', 'Errors', 'Duration', 'Throttles'] } },
  { id: 'aws-rds', name: 'Amazon RDS', logo: 'amazonrds', categories: ['AWS', 'Data Stores'], status: 'warning', warning: 'Enhanced Monitoring is disabled on 2 instances.', collect: { kind: 'cloudwatch', namespace: 'AWS/RDS', metrics: ['CPUUtilization', 'DatabaseConnections', 'FreeStorageSpace', 'ReadLatency'] } },
  { id: 'aws-s3', name: 'Amazon S3', logo: 'amazons3', categories: ['AWS', 'Data Stores'], status: 'installed', collect: { kind: 'cloudwatch', namespace: 'AWS/S3', metrics: ['BucketSizeBytes', 'NumberOfObjects', 'AllRequests'] } },
  { id: 'kubernetes', name: 'Kubernetes', categories: ['Kubernetes', 'Containers', 'Orchestration'], status: 'installed', logs: 'kubernetes', collect: { kind: 'otel', receiver: 'k8s_cluster', fields: ['auth_type: serviceAccount', 'node_conditions_to_report: [Ready, MemoryPressure]'], metrics: K8S } },
  { id: 'nginx', name: 'NGINX', categories: ['Web', 'Network'], status: 'installed', logs: 'file:/var/log/nginx/*.log', collect: { kind: 'otel', receiver: 'nginx', fields: ['endpoint: http://localhost:80/nginx_status'], metrics: NGINX } },
  { id: 'postgresql', name: 'PostgreSQL', categories: ['Data Stores'], status: 'installed', logs: 'file:/var/log/postgresql/*.log', collect: { kind: 'otel', receiver: 'postgresql', fields: ['endpoint: ${env:PG_ENDPOINT}', 'username: observecraft', 'password: ${env:PG_PASSWORD}', 'tls:\n      insecure: true'], metrics: POSTGRES } },
  { id: 'mongodb', name: 'MongoDB', categories: ['Data Stores'], status: 'installed', logs: 'file:/var/log/mongodb/*.log', collect: { kind: 'otel', receiver: 'mongodb', fields: ['hosts:\n      - endpoint: ${env:MONGO_ENDPOINT}', 'username: observecraft', 'password: ${env:MONGO_PASSWORD}'], metrics: MONGO } },
  { id: 'kafka', name: 'Kafka', logo: 'apachekafka', categories: ['Message Queues'], status: 'installed', collect: { kind: 'otel', receiver: 'kafkametrics', fields: ['brokers: [${env:KAFKA_BROKERS}]', 'protocol_version: 2.0.0', 'scrapers: [brokers, topics, consumers]'], metrics: KAFKA } },
  { id: 'rabbitmq', name: 'RabbitMQ', categories: ['Message Queues'], status: 'installed', collect: { kind: 'otel', receiver: 'rabbitmq', fields: ['endpoint: http://localhost:15672', 'username: ${env:RABBITMQ_USER}', 'password: ${env:RABBITMQ_PASSWORD}'], metrics: RABBIT } },
  { id: 'opentelemetry', name: 'OpenTelemetry', categories: ['Tracing', 'Metrics', 'Log Collection'], status: 'installed', collect: { kind: 'otlp', metrics: [] } },
  { id: 'slack', name: 'Slack', categories: ['Collaboration', 'Notifications', 'Alerting'], status: 'installed', collect: { kind: 'notify', settings: NOTIFY_WEBHOOK } },
  { id: 'pagerduty', name: 'PagerDuty', categories: ['Alerting', 'Incidents'], status: 'installed', collect: { kind: 'notify', settings: [{ key: 'routing_key', label: 'Integration (routing) key', placeholder: 'R0…', secret: true }] } },
  { id: 'github', name: 'GitHub', categories: ['Source Control', 'Developer Tools'], status: 'installed', collect: { kind: 'notify', settings: [{ key: 'app_installation', label: 'GitHub App installation ID', placeholder: '12345678' }] } },
  // ---- available
  { id: 'aws-dynamodb', name: 'Amazon DynamoDB', logo: 'amazondynamodb', categories: ['AWS', 'Data Stores'], collect: { kind: 'cloudwatch', namespace: 'AWS/DynamoDB', metrics: ['ConsumedReadCapacityUnits', 'ConsumedWriteCapacityUnits', 'ThrottledRequests'] } },
  { id: 'aws-ec2', name: 'Amazon EC2', logo: 'amazonec2', categories: ['AWS', 'OS & System'], collect: { kind: 'cloudwatch', namespace: 'AWS/EC2', metrics: ['CPUUtilization', 'NetworkIn', 'NetworkOut', 'StatusCheckFailed'] } },
  { id: 'aws-eks', name: 'Amazon EKS', logo: 'amazoneks', categories: ['AWS', 'Kubernetes', 'Containers'], collect: { kind: 'cloudwatch', namespace: 'ContainerInsights', metrics: ['node_cpu_utilization', 'pod_number_of_container_restarts'] } },
  { id: 'aws-ecs', name: 'Amazon ECS', logo: 'amazonecs', categories: ['AWS', 'Containers'], collect: { kind: 'cloudwatch', namespace: 'AWS/ECS', metrics: ['CPUUtilization', 'MemoryUtilization'] } },
  { id: 'azure', name: 'Microsoft Azure', logo: 'tile:azure', categories: ['Azure', 'Cost Management'], collect: { kind: 'otel', receiver: 'azuremonitor', fields: ['subscription_ids: [${env:AZURE_SUBSCRIPTION_ID}]', 'tenant_id: ${env:AZURE_TENANT_ID}', 'client_id: ${env:AZURE_CLIENT_ID}', 'client_secret: ${env:AZURE_CLIENT_SECRET}'], metrics: [['azure.vm.percentage_cpu', 'gauge', 'VM CPU percentage'], ['azure.storage.transactions', 'count', 'Storage transactions']] } },
  { id: 'google-cloud', name: 'Google Cloud Platform', logo: 'googlecloud', categories: ['Google Cloud', 'Cost Management'], collect: { kind: 'otel', receiver: 'googlecloudmonitoring', fields: ['project_id: ${env:GCP_PROJECT}', 'metrics_list:\n      - metric_name: compute.googleapis.com/instance/cpu/utilization'], metrics: [['gcp.compute.instance.cpu.utilization', 'gauge', 'Instance CPU utilization', '1']] } },
  { id: 'oracle', name: 'Oracle Database', logo: 'oracle', categories: ['Oracle', 'Data Stores'], collect: { kind: 'otel', receiver: 'oracledb', fields: ['datasource: oracle://observecraft:${env:ORACLE_PASSWORD}@db-host:1521/ORCL'], metrics: [['oracledb.sessions.usage', 'gauge', 'Session usage'], ['oracledb.cpu_time', 'count', 'CPU time', 's'], ['oracledb.tablespace_size.usage', 'gauge', 'Tablespace usage', 'By']] } },
  { id: 'openai', name: 'OpenAI', categories: ['AI/ML'], collect: { kind: 'otlp', metrics: LLM } },
  { id: 'anthropic', name: 'Anthropic', categories: ['AI/ML'], collect: { kind: 'otlp', metrics: LLM } },
  { id: 'huggingface', name: 'Hugging Face TGI', categories: ['AI/ML'], collect: { kind: 'prometheus', target: 'tgi:80/metrics', metrics: prom('tgi', [['request_count', 'count', 'Requests'], ['request_duration', 'distribution', 'Request duration'], ['queue_size', 'gauge', 'Queued requests']]) } },
  { id: 'ollama', name: 'Ollama', categories: ['AI/ML'], collect: { kind: 'otlp', metrics: LLM } },
  { id: 'opsgenie', name: 'Opsgenie', categories: ['Alerting', 'Incidents'], collect: { kind: 'notify', settings: [{ key: 'api_key', label: 'API key', placeholder: '••••', secret: true }] } },
  { id: 'microsoft-teams', name: 'Microsoft Teams', logo: 'tile:teams', categories: ['Collaboration', 'Notifications'], collect: { kind: 'notify', settings: NOTIFY_WEBHOOK } },
  { id: 'jira', name: 'Jira', categories: ['Collaboration', 'Incidents'], collect: { kind: 'notify', settings: [{ key: 'site', label: 'Site URL', placeholder: 'https://your-team.atlassian.net' }, { key: 'token', label: 'API token', placeholder: '••••', secret: true }] } },
  { id: 'gitlab', name: 'GitLab', categories: ['Source Control', 'Developer Tools'], collect: { kind: 'notify', settings: [{ key: 'token', label: 'Access token', placeholder: 'glpat-…', secret: true }] } },
  { id: 'terraform', name: 'Terraform', categories: ['Configuration & Deployment', 'Automation'], collect: { kind: 'notify', settings: [{ key: 'org', label: 'Organization', placeholder: 'acme' }, { key: 'token', label: 'API token', placeholder: '••••', secret: true }] } },
  { id: 'ansible', name: 'Ansible', categories: ['Configuration & Deployment', 'Automation'], collect: { kind: 'notify', settings: NOTIFY_WEBHOOK } },
  { id: 'docker', name: 'Docker', categories: ['Containers'], logs: 'docker', collect: { kind: 'otel', receiver: 'docker_stats', fields: ['endpoint: unix:///var/run/docker.sock'], metrics: [['container.cpu.utilization', 'gauge', 'Container CPU', '1'], ['container.memory.usage.total', 'gauge', 'Container memory', 'By'], ['container.network.io.usage.rx_bytes', 'count', 'Bytes received', 'By']] } },
  { id: 'containerd', name: 'containerd', categories: ['Containers'], collect: { kind: 'prometheus', target: 'localhost:1338', metrics: prom('containerd', [['cpu_total', 'count', 'CPU time'], ['memory_usage_bytes', 'gauge', 'Memory usage']]) } },
  { id: 'mysql', name: 'MySQL', categories: ['Data Stores'], logs: 'file:/var/log/mysql/*.log', collect: { kind: 'otel', receiver: 'mysql', fields: ['endpoint: ${env:MYSQL_ENDPOINT}', 'username: observecraft', 'password: ${env:MYSQL_PASSWORD}'], metrics: [['mysql.connection.count', 'count', 'Connections'], ['mysql.operations', 'count', 'Operations'], ['mysql.buffer_pool.usage', 'gauge', 'Buffer pool usage', 'By']] } },
  { id: 'elasticsearch', name: 'Elasticsearch', categories: ['Data Stores'], collect: { kind: 'otel', receiver: 'elasticsearch', fields: ['endpoint: http://localhost:9200'], metrics: [['elasticsearch.cluster.health', 'gauge', 'Cluster health'], ['elasticsearch.node.operations.completed', 'count', 'Completed operations']] } },
  { id: 'cassandra', name: 'Cassandra', logo: 'apachecassandra', categories: ['Data Stores'], collect: { kind: 'otel', receiver: 'jmx', fields: ['jar_path: /opt/opentelemetry-jmx-metrics.jar', 'endpoint: localhost:7199', 'target_system: cassandra'], metrics: [['cassandra.client.request.count', 'count', 'Client requests'], ['cassandra.storage.load.count', 'gauge', 'Storage load', 'By']] } },
  { id: 'memcached', name: 'Memcached', logo: 'tile:memcached', categories: ['Caching', 'Data Stores'], collect: { kind: 'otel', receiver: 'memcached', fields: ['endpoint: localhost:11211'], metrics: [['memcached.operations', 'count', 'Operations'], ['memcached.current_items', 'gauge', 'Items stored']] } },
  { id: 'haproxy', name: 'HAProxy', logo: 'tile:haproxy', categories: ['Network', 'Web'], collect: { kind: 'otel', receiver: 'haproxy', fields: ['endpoint: file:///var/run/haproxy.ipc'], metrics: [['haproxy.requests.total', 'count', 'HTTP requests'], ['haproxy.connections.total', 'count', 'Connections']] } },
  { id: 'istio', name: 'Istio', categories: ['Kubernetes', 'Network'], collect: { kind: 'prometheus', target: 'istiod.istio-system:15014', metrics: prom('istio', [['requests_total', 'count', 'Requests'], ['request_duration_milliseconds', 'distribution', 'Request duration']]) } },
  { id: 'linux', name: 'Linux Host', categories: ['OS & System'], logs: 'journald', os: ['Linux'], collect: { kind: 'otel', receiver: 'hostmetrics', fields: ['scrapers:\n      cpu: {}\n      memory: {}\n      filesystem: {}\n      network: {}'], metrics: HOST } },
  { id: 'vault', name: 'HashiCorp Vault', logo: 'vault', categories: ['Security'], collect: { kind: 'prometheus', target: 'vault:8200/v1/sys/metrics?format=prometheus', metrics: prom('vault', [['core_unsealed', 'gauge', 'Unsealed state'], ['token_count', 'gauge', 'Active tokens']]) } },
  { id: 'cloudflare', name: 'Cloudflare', categories: ['Network', 'Security'], collect: { kind: 'notify', settings: [{ key: 'token', label: 'API token', placeholder: '••••', secret: true }] } },
  { id: 'jenkins', name: 'Jenkins', categories: ['Developer Tools', 'Automation'], collect: { kind: 'prometheus', target: 'jenkins:8080/prometheus', metrics: prom('jenkins', [['builds_duration_milliseconds_summary', 'distribution', 'Build duration'], ['executor_in_use_value', 'gauge', 'Executors in use']]) } },
  { id: 'github-actions', name: 'GitHub Actions', logo: 'githubactions', categories: ['Developer Tools', 'Testing'], collect: { kind: 'notify', settings: [{ key: 'app_installation', label: 'GitHub App installation ID', placeholder: '12345678' }] } },
  { id: 'sentry', name: 'Sentry', categories: ['Developer Tools'], collect: { kind: 'notify', settings: NOTIFY_WEBHOOK } },
  { id: 'helm', name: 'Helm', categories: ['Kubernetes', 'Configuration & Deployment'], collect: { kind: 'otel', receiver: 'k8sobjects', fields: ['objects:\n      - name: secrets\n        label_selector: owner=helm'], metrics: [['helm.release.status', 'gauge', 'Release status by chart']] } },
  { id: 'prometheus', name: 'Prometheus', categories: ['Metrics'], collect: { kind: 'prometheus', target: 'my-app:9090', metrics: [] } },
  { id: 'nodejs', name: 'Node.js', logo: 'nodedotjs', categories: ['Languages', 'Tracing'], collect: { kind: 'otlp', metrics: RUNTIME('nodejs') } },
  { id: 'python', name: 'Python', categories: ['Languages', 'Tracing'], collect: { kind: 'otlp', metrics: RUNTIME('python') } },
  { id: 'go', name: 'Go', categories: ['Languages', 'Tracing'], collect: { kind: 'otlp', metrics: RUNTIME('go') } },
  { id: 'java', name: 'Java', logo: 'openjdk', categories: ['Languages', 'Tracing'], collect: { kind: 'otlp', metrics: RUNTIME('jvm') } },
  { id: 'consul', name: 'Consul', categories: ['Network', 'Configuration & Deployment'], collect: { kind: 'prometheus', target: 'consul:8500/v1/agent/metrics?format=prometheus', metrics: prom('consul', [['raft_leader', 'gauge', 'Raft leader'], ['catalog_services', 'gauge', 'Registered services']]) } },
  { id: 'etcd', name: 'etcd', categories: ['Kubernetes', 'Data Stores'], collect: { kind: 'prometheus', target: 'etcd:2379', metrics: prom('etcd', [['server_has_leader', 'gauge', 'Has leader'], ['disk_wal_fsync_duration_seconds', 'distribution', 'WAL fsync duration']]) } },
  { id: 'fluent-bit', name: 'Fluent Bit', logo: 'fluentbit', categories: ['Log Collection'], collect: { kind: 'prometheus', target: 'localhost:2020/api/v1/metrics/prometheus', metrics: prom('fluentbit', [['input_records_total', 'count', 'Input records'], ['output_errors_total', 'count', 'Output errors']]) } },
  { id: 'snowflake', name: 'Snowflake', categories: ['Data Stores', 'Cost Management'], collect: { kind: 'otel', receiver: 'snowflake', fields: ['account: ${env:SNOWFLAKE_ACCOUNT}', 'username: observecraft', 'password: ${env:SNOWFLAKE_PASSWORD}', 'warehouse: OBSERVABILITY_WH'], metrics: [['snowflake.query.executed', 'count', 'Queries executed'], ['snowflake.billing.warehouse.total_credit.total', 'gauge', 'Credits used']] } },
  { id: 'databricks', name: 'Databricks', categories: ['Data Stores', 'AI/ML'], collect: { kind: 'notify', settings: [{ key: 'host', label: 'Workspace URL', placeholder: 'https://adb-….azuredatabricks.net' }, { key: 'token', label: 'Access token', placeholder: '••••', secret: true }] } },
  { id: 'okta', name: 'Okta', categories: ['Security'], collect: { kind: 'notify', settings: [{ key: 'domain', label: 'Okta domain', placeholder: 'acme.okta.com' }, { key: 'token', label: 'API token', placeholder: '••••', secret: true }] } },
  { id: 'palo-alto', name: 'Palo Alto Networks Firewall', logo: 'paloaltonetworks', categories: ['Security', 'Network'], logs: 'syslog', collect: { kind: 'notify', settings: [{ key: 'syslog_port', label: 'Syslog port', placeholder: '514' }] } },
  { id: 'cypress', name: 'Cypress', categories: ['Testing'], collect: { kind: 'otlp', metrics: [['test.run.duration', 'distribution', 'Run duration', 's'], ['test.failures', 'count', 'Failed tests']] } },
  { id: 'selenium', name: 'Selenium Grid', logo: 'selenium', categories: ['Testing'], collect: { kind: 'prometheus', target: 'selenium-hub:4444', metrics: prom('selenium_grid', [['sessions_queued', 'gauge', 'Queued sessions'], ['node_count', 'gauge', 'Nodes']]) } },
  { id: 'airflow', name: 'Airflow', logo: 'apacheairflow', categories: ['Automation', 'Orchestration'], collect: { kind: 'otel', receiver: 'statsd', fields: ['endpoint: localhost:8125'], metrics: [['airflow.dagrun.duration.success', 'distribution', 'Successful DAG run duration', 'ms'], ['airflow.ti_failures', 'count', 'Task instance failures']] } },
  { id: 'circleci', name: 'CircleCI', categories: ['Developer Tools', 'Testing'], collect: { kind: 'notify', settings: [{ key: 'token', label: 'API token', placeholder: '••••', secret: true }] } },
  { id: 'zendesk', name: 'Zendesk', categories: ['Collaboration'], collect: { kind: 'notify', settings: [{ key: 'subdomain', label: 'Subdomain', placeholder: 'acme' }, { key: 'token', label: 'API token', placeholder: '••••', secret: true }] } },
  { id: 'twilio', name: 'Twilio', categories: ['Alerting', 'Notifications'], collect: { kind: 'notify', settings: [{ key: 'sid', label: 'Account SID', placeholder: 'AC…' }, { key: 'token', label: 'Auth token', placeholder: '••••', secret: true }] } },
  { id: 'stripe', name: 'Stripe', categories: ['Developer Tools'], collect: { kind: 'notify', settings: [{ key: 'key', label: 'Restricted API key', placeholder: 'rk_live_…', secret: true }] } },
];

// ---------------------------------------------------------------- derivation

const COLLECTOR_VERSION = '0.112.0';
const HOSTS = ['i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003', 'i-0a1b2c3d4e5f60006', 'ip-10-42-3-17', 'ip-10-42-7-201'];

/** Session-local install state (the mock's "database"). */
const statusOverride = new Map<string, IntegrationStatus>();

function statusOf(e: Entry): IntegrationStatus {
  return statusOverride.get(e.id) ?? e.status ?? 'available';
}

function summary(e: Entry): IntegrationSummary {
  const status = statusOf(e);
  return {
    id: e.id,
    name: e.name,
    publisher: 'ObserveCraft',
    logo: e.logo ?? e.id,
    status,
    categories: e.categories,
    description: `Monitor the health and performance of ${e.name}.`,
    warning: status === 'warning' ? e.warning : undefined,
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
    case 'cloudwatch':
      out.push({
        target: 'otel-collector',
        title: 'OpenTelemetry Collector — CloudWatch metrics',
        filename: file(''),
        language: 'yaml',
        content: otelFragment(
          e.id,
          `awscloudwatchmetrics/${e.id}`,
          [
            '    region: ${env:AWS_REGION}',
            '    poll_interval: 5m',
            '    metrics:',
            '      named:',
            ...c.metrics.flatMap((m) => [`        - namespace: "${c.namespace}"`, `          metric_name: "${m}"`, '          period: "5m"', '          aws_aggregation: "Average"']),
          ].join('\n'),
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
  if (c.kind === 'notify') return [];
  if (c.kind === 'cloudwatch') {
    const ns = c.namespace.replace(/^AWS\//, '').toLowerCase();
    return c.metrics.map((m) => ({ name: `aws.${ns}.${m.replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase()}`, type: 'gauge', description: `${c.namespace} ${m}` }));
  }
  return c.metrics.map(([name, type, description, unit]) => ({ name, type, description, unit }));
}

function hostsFor(e: Entry, status: IntegrationStatus): IntegrationHost[] {
  if (status === 'available' || e.collect.kind === 'notify' || e.collect.kind === 'cloudwatch') return [];
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
  const dataTypes = [
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
        : c.kind === 'cloudwatch'
          ? `CloudWatch (${c.namespace})`
          : c.kind === 'otlp'
            ? 'OTLP from the OpenTelemetry SDK in your application'
            : 'the provider API — no collector changes are required';
  const steps =
    c.kind === 'notify'
      ? [`Fill in the ${e.name} settings below.`, 'Click Install. ObserveCraft validates the credentials.', `Reference @${e.id} in monitor notifications.`]
      : c.kind === 'otlp'
        ? ['Add the OpenTelemetry SDK and auto-instrumentation to your service.', 'Point OTEL_EXPORTER_OTLP_ENDPOINT at the local collector (port 4317).', 'Click Install to enable the bundled dashboards and monitors.']
        : [
            `Click Install. The fleet manager adds the configuration below to collectors on matching hosts.`,
            'Set any referenced environment variables (credentials, endpoints) via your secret store.',
            `Data appears within a few minutes under the "${e.id}" source.`,
          ];
  return {
    ...s,
    overview: `${e.about ? `${e.about} ` : ''}This integration collects ${e.name} telemetry through ${how}.`,
    offerings: ['Integration'],
    supportedOs: e.os ?? ['Linux', 'Windows', 'macOS'],
    dataTypes,
    minCollectorVersion: COLLECTOR_VERSION,
    setupSteps: steps,
    configs: configsFor(e),
    settings: c.kind === 'notify' ? c.settings : [],
    metrics: metricsFor(e),
    logs: e.logs ? `Logs are collected by Vector and tagged source:${e.id}.` : null,
    hosts: hostsFor(e, s.status),
    resources:
      c.kind === 'notify'
        ? []
        : [
            { kind: 'dashboard', name: `${e.name} - Overview`, description: 'Throughput, errors and saturation at a glance.' },
            { kind: 'monitor', name: `${e.name} is not reporting`, description: 'No data from the integration for 10 minutes.' },
            { kind: 'monitor', name: `${e.name} error rate is high`, description: 'Error rate above 5% for 5 minutes.' },
          ],
    releaseNotes: [
      { version: '1.3.0', date: '2026-09-02', notes: ['Added default monitors.', `Updated for collector ${COLLECTOR_VERSION}.`] },
      { version: '1.2.1', date: '2026-06-18', notes: ['Fixed unit metadata on several metrics.'] },
      { version: '1.0.0', date: '2026-01-10', notes: ['Initial release.'] },
    ],
  };
}

function find(id: string): Entry {
  const e = CATALOG.find((x) => x.id === id);
  if (!e) throw new Error(`Integration ${id} not found`);
  return e;
}

// ---------------------------------------------------------------- endpoints

export function searchCatalog(req: CatalogRequest): Promise<CatalogResponse> {
  const q = req.query.trim().toLowerCase();
  const integrations = CATALOG.map(summary)
    .filter((s) => (req.status === 'all' ? true : req.status === 'installed' ? s.status === 'installed' || s.status === 'warning' : s.status === req.status))
    .filter((s) => !req.category || s.categories.includes(req.category))
    .filter((s) => !q || s.name.toLowerCase().includes(q) || s.categories.some((c) => c.toLowerCase().includes(q)))
    .sort((a, b) => a.name.localeCompare(b.name));
  return delay({ integrations });
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

// ---------------------------------------------------------------- marketplace

const APPS: MarketplaceApp[] = [
  { id: 'airtable', name: 'Airtable', partner: 'Northwind Labs', logo: 'airtable', offering: 'Integration', categories: ['Collaboration'], description: 'Base activity, automation runs and API usage.' },
  { id: 'threatstream', name: 'Threat Intel Stream', partner: 'Northwind Labs', logo: 'tile:threatstream', offering: 'Integration', categories: ['Security'], description: 'Enrich security signals with threat intelligence indicators.' },
  { id: 'ansible-aap', name: 'Ansible Automation Platform', partner: 'Bluepeak Data', logo: 'ansible', offering: 'Integration', categories: ['Automation', 'Configuration & Deployment'], description: 'Job runs, inventories and host failures from AAP.' },
  { id: 'iotdb', name: 'Apache IoTDB', partner: 'Bluepeak Data', logo: 'tile:iotdb', offering: 'Integration', categories: ['Data Stores'], description: 'Time-series storage engine health and ingestion rate.' },
  { id: 'saas-posture', name: 'SaaS Posture Guard', partner: 'Cobalt Systems', logo: 'tile:posture', offering: 'Software License', categories: ['Security'], description: 'SaaS security posture findings as ObserveCraft events.' },
  { id: 'asset-inventory', name: 'Asset Inventory', partner: 'Northwind Labs', logo: 'tile:inventory', offering: 'Integration', categories: ['Security', 'Network'], description: 'Device inventory and risk scores.' },
  { id: 'bamboo', name: 'Atlassian Bamboo', partner: 'Bluepeak Data', logo: 'bamboo', offering: 'Integration', categories: ['Developer Tools', 'Testing'], description: 'Build plans, durations and failure rates.' },
  { id: 'atomwatch', name: 'AtomWatch', partner: 'Atomwise Ltd', logo: 'tile:atomwatch', offering: 'Software License', categories: ['Testing'], description: 'Synthetic transaction monitoring for legacy apps.' },
  { id: 'automic', name: 'Workload Automation', partner: 'Bluepeak Data', logo: 'tile:automic', offering: 'Integration', categories: ['Automation'], description: 'Job scheduling health and SLA breaches.' },
  { id: 'automox', name: 'Automox', partner: 'Northwind Labs', logo: 'tile:automox', offering: 'Integration', categories: ['OS & System', 'Security'], description: 'Patch compliance across endpoints.' },
  { id: 'avd', name: 'Azure Virtual Desktop', partner: 'Bluepeak Data', logo: 'tile:avd', offering: 'Integration', categories: ['Azure'], description: 'Session hosts, connection quality and user sessions.' },
  { id: 'backup-automator', name: 'Backup Automator', partner: 'Bluepeak Data', logo: 'tile:backup', offering: 'Software License', categories: ['Automation'], description: 'Back up dashboards, monitors and pipelines on a schedule.' },
  { id: 'barracuda-waf', name: 'Barracuda WAF', partner: 'Northwind Labs', logo: 'tile:waf', offering: 'Integration', categories: ['Security', 'Network'], description: 'Web application firewall events and blocked requests.' },
  { id: 'box', name: 'Box', partner: 'Bluepeak Data', logo: 'box', offering: 'Integration', categories: ['Collaboration', 'Security'], description: 'Admin events and file activity audit logs.' },
  { id: 'circleci-insights', name: 'CircleCI Insights', partner: 'Cobalt Systems', logo: 'circleci', offering: 'Integration', categories: ['Developer Tools', 'Testing'], description: 'Workflow durations, flaky tests and credits used.' },
  { id: 'onboarding', name: 'Observability Onboarding', partner: 'Lumen Partners', logo: 'tile:onboarding', offering: 'Professional Services', categories: ['Automation'], description: 'Two-week guided rollout of collectors, dashboards and monitors.' },
  { id: 'snowflake-cost', name: 'Snowflake Cost Explorer', partner: 'Cobalt Systems', logo: 'snowflake', offering: 'Software License', categories: ['Cost Management', 'Data Stores'], description: 'Warehouse credit usage broken down by team.' },
  { id: 'zoom-rooms', name: 'Zoom Rooms', partner: 'Northwind Labs', logo: 'zoom', offering: 'Integration', categories: ['Collaboration'], description: 'Room health, meeting quality and device status.' },
  { id: 'twilio-flex', name: 'Twilio Flex', partner: 'Bluepeak Data', logo: 'twilio', offering: 'Integration', categories: ['Collaboration'], description: 'Contact-center queues, wait times and agent activity.' },
  { id: 'migration', name: 'Monitoring Migration', partner: 'Lumen Partners', logo: 'tile:migration', offering: 'Professional Services', categories: ['Configuration & Deployment'], description: 'Migrate dashboards and alerts from your previous tool.' },
];

export function searchMarketplace(req: MarketplaceRequest): Promise<MarketplaceApp[]> {
  const q = req.query.trim().toLowerCase();
  return delay(
    APPS.filter((a) => !req.category || a.categories.includes(req.category))
      .filter((a) => !q || `${a.name} ${a.partner}`.toLowerCase().includes(q))
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
}

/** Static catalog metadata, without the demo installation state or generated hosts. */
export function catalogMetadata(): IntegrationSummary[] {
  return CATALOG.map((e) => ({ ...summary(e), status: 'available', warning: undefined }));
}

export function integrationMetadata(id: string): IntegrationDetail {
  const e = find(id);
  return {
    ...detail(e),
    status: 'available',
    warning: undefined,
    hosts: [],
    resources: [],
    releaseNotes: [],
    setupSteps:
      e.collect.kind === 'notify'
        ? ['Configure a supported notification channel in Organization Settings. This catalog does not provision provider connections.']
        : [
            'Apply the configuration template to your own collector and supply the required endpoints and credentials.',
            'Add the receiver to the appropriate collector service pipeline and configure an exporter to the ObserveCraft intake with your organization ingest key.',
            'Restart or reload the collector. Detection is based on matching metrics received during the last 24 hours.',
          ],
  };
}
