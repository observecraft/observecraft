// How to run an Agent check against a workload on Kubernetes: the Datadog Agent's Autodiscovery annotations on the
// workload's pods (`ad.datadoghq.com/<container>.checks`), or a Cluster Agent cluster check for checks that run once
// per cluster, per check, for the ways the software is usually deployed; for managed databases outside the cluster
// (Azure Database for PostgreSQL, RDS), a cluster check the Datadog Helm chart's cluster checks runners run. Our own
// words and examples; shown on the integration's Configure tab below the Agent configuration form.

export interface KubernetesHintVariant {
  /** How the software is deployed (its Helm chart, the Omnibus image, a managed control plane). */
  label: string;
  /** Where the configuration goes and what to set on the software's side. */
  text: string;
  /** What the YAML is: pod annotations, the chart's values, the operator's resources; none when the Agent runs the check by itself. */
  filename?: string;
  /** The pod annotations, or the manifest or values carrying them (YAML). */
  code?: string;
}

export interface KubernetesHint {
  check: string;
  variants: KubernetesHintVariant[];
}

const POD_ANNOTATIONS = 'Pod annotations';
const CLUSTER_CHECK_VALUES = 'values.yaml (Datadog chart)';

/** `text` with every line indented by `spaces`. */
const indent = (text: string, spaces: number) =>
  text
    .split('\n')
    .map((l) => ' '.repeat(spaces) + l)
    .join('\n');

/** `ad.datadoghq.com/<container>.checks` with the check's configuration, as pod annotations in YAML. */
export function checkAnnotation(container: string, check: string, config: { init_config?: object; instances: object[] }): string {
  const body = JSON.stringify({ [check]: { init_config: config.init_config ?? {}, instances: config.instances } }, null, 2);
  return `ad.datadoghq.com/${container}.checks: |\n${indent(body, 2)}`;
}

/** The JMX check of the Agent's JMXFetch on the pod's JMX port. */
const jmxCheck = (port: number, init: object = {}) => ({
  init_config: { is_jmx: true, collect_default_metrics: true, ...init },
  instances: [{ host: '%%host%%', port }],
});

/** Said by every JMX variant: the Agent runs JMX checks only in its JMX image. */
const JMX_AGENT = 'JMX checks run in the Agent’s JMX image: set `agents.image.tagSuffix: jmx` in the Datadog chart’s values (Datadog Operator: `override.nodeAgent.image.jmxEnabled: true`).';

/** A Cluster Agent cluster check in the Datadog chart's values (`clusterAgent.confd`): run once per cluster by one Agent. */
function clusterCheck(check: string, instances: object[], init: object = {}): string {
  const body = `cluster_check: true\ninit_config: ${JSON.stringify(init)}\ninstances:\n${instances.map((i) => `  - ${JSON.stringify(i)}`).join('\n')}`;
  return `datadog:\n  clusterChecks:\n    enabled: true\nclusterAgent:\n  confd:\n    ${check}.yaml: |\n${indent(body, 6)}`;
}

/** `ad.datadoghq.com/<container>.logs`: the container's logs tagged with the integration's source (its pipeline's filter). */
export function logsAnnotation(container: string, source: string, service: string): string {
  return `ad.datadoghq.com/${container}.logs: '${JSON.stringify([{ source, service }])}'`;
}

/** The Datadog Helm chart's values running a check on the cluster checks runners (`clusterAgent.confd`), plus `extra` values. */
export function clusterCheckValues(check: string, instance: object, extra = ''): string {
  const body = JSON.stringify({ cluster_check: true, init_config: {}, instances: [instance] }, null, 2);
  return `clusterChecksRunner:\n  enabled: true\n${extra}clusterAgent:\n  confd:\n    ${check}.yaml: |\n${indent(body, 6)}`;
}

/** Plain data as YAML (objects, lists, strings, numbers, booleans), indented by `pad`. */
function toYaml(v: unknown, pad = ''): string {
  if (Array.isArray(v)) return v.map((x) => `${pad}- ${toYaml(x, `${pad}  `).trimStart()}`).join('\n');
  if (v && typeof v === 'object')
    return Object.entries(v)
      .map(([k, x]) => (x && typeof x === 'object' && Object.keys(x).length ? `${pad}${k}:\n${toYaml(x, `${pad}  `)}` : `${pad}${k}: ${x && typeof x === 'object' ? '{}' : toYaml(x)}`))
      .join('\n');
  return typeof v === 'string' ? JSON.stringify(v) : String(v);
}

/** A cluster check in the Datadog Helm chart's values: the Cluster Agent runs it against a Service's endpoints. */
export function endpointsClusterCheck(check: string, service: { name: string; namespace: string }, instances: object[], init_config: object = {}): string {
  const config = toYaml({ advanced_ad_identifiers: [{ kube_endpoints: service }], cluster_check: true, init_config, instances }, '      ');
  return `clusterAgent:\n  confd:\n    ${check}.yaml: |-\n${config}`;
}

/** Reading a password from a Kubernetes secret: the Agent's secret backend (`ENC[k8s_secret@namespace/name/key]`). */
const SECRET_NOTE =
  'The password is read from a Kubernetes secret by the Agent’s secret backend: set `datadog.secretBackend.command: /readsecret_multiple_providers.sh` in the Datadog chart’s values (it lets the Agent read secrets; `datadog.secretBackend.roles` narrows that to the ones named).';

/** The runner metrics the gitlab_runner check keeps (its sample configuration's list, shortened to the runner's own). */
const RUNNER_METRICS = [
  'ci_runner_builds',
  'ci_runner_errors',
  'gitlab_runner_api_request_duration_seconds',
  'gitlab_runner_autoscaling_machine_states',
  'gitlab_runner_errors_total',
  'gitlab_runner_job_queue_duration_seconds',
  'gitlab_runner_jobs',
  'gitlab_runner_version_info',
  'process_cpu_seconds_total',
  'process_resident_memory_bytes',
];

const HINTS: KubernetesHint[] = [
  {
    check: 'gitlab',
    variants: [
      {
        label: 'GitLab Helm chart (gitlab/gitlab)',
        text: 'Put the annotations on the webservice pods with the chart’s `gitlab.webservice.annotations` value; the chart already serves the Rails metrics on port 8083 (`gitlab.webservice.metrics.enabled`). Gitaly runs in its own pods: point `gitaly_server_endpoint` at its service. Replace `gitlab` in the service names with your release’s name.',
        filename: POD_ANNOTATIONS,
        code: checkAnnotation('webservice', 'gitlab', {
          instances: [
            {
              gitlab_url: 'http://%%host%%:8181',
              openmetrics_endpoint: 'http://%%host%%:8083/metrics',
              gitaly_server_endpoint: 'http://gitlab-gitaly.gitlab.svc:9236/metrics',
            },
          ],
        }),
      },
      {
        label: 'Omnibus image (gitlab/gitlab-ee) in a pod',
        text: "Annotate the pod running the Omnibus container (here named `gitlab`). Let the Agent read the metrics: add the pod network to `gitlab_rails['monitoring_whitelist']` and have Gitaly listen on all addresses (`gitaly['configuration'] = { prometheus_listen_addr: '0.0.0.0:9236' }`) in `GITLAB_OMNIBUS_CONFIG`, then restart GitLab.",
        filename: POD_ANNOTATIONS,
        code: checkAnnotation('gitlab', 'gitlab', {
          instances: [{ gitlab_url: 'http://%%host%%', openmetrics_endpoint: 'http://%%host%%/-/metrics', gitaly_server_endpoint: 'http://%%host%%:9236/metrics' }],
        }),
      },
    ],
  },
  {
    check: 'gitlab_runner',
    variants: [
      {
        label: 'GitLab Runner Helm chart (gitlab/gitlab-runner)',
        text: 'Turn the runner’s metrics on (`metrics.enabled: true`, port 9252) and put the annotations in the chart’s `podAnnotations` value. The container is named after the release (here `gitlab-runner`); `gitlab_url` is the GitLab the runner is registered with, for the check’s service check.',
        filename: POD_ANNOTATIONS,
        code: checkAnnotation('gitlab-runner', 'gitlab_runner', {
          init_config: { allowed_metrics: RUNNER_METRICS },
          instances: [{ prometheus_endpoint: 'http://%%host%%:9252/metrics', gitlab_url: 'https://gitlab.example.com' }],
        }),
      },
    ],
  },
  {
    check: 'kafka',
    variants: [
      {
        label: 'Strimzi (Kafka and KafkaNodePool resources, KRaft)',
        text: `\`jmxOptions: {}\` makes Strimzi open JMX on port 9999 of every Kafka pod, without authentication. Annotate the pods through each node pool’s pod template (the container is named \`kafka\`); controllers report the KRaft metrics, brokers the rest. Strimzi’s \`metricsConfig\` (the Prometheus JMX exporter) is for Prometheus: its metric names are not the ones the Kafka dashboards read. ${JMX_AGENT}`,
        filename: 'Kafka and KafkaNodePool',
        code: `apiVersion: kafka.strimzi.io/v1
kind: Kafka
metadata:
  name: my-cluster
spec:
  kafka:
    jmxOptions: {}
    # listeners, config, … as before
---
apiVersion: kafka.strimzi.io/v1
kind: KafkaNodePool
metadata:
  name: brokers
  labels:
    strimzi.io/cluster: my-cluster
spec:
  roles: [broker]
  # replicas, storage, … as before
  template:
    pod:
      metadata:
        annotations:
${indent(checkAnnotation('kafka', 'kafka', jmxCheck(9999)), 10)}`,
      },
      {
        label: 'Bitnami Kafka chart (bitnami/kafka, KRaft)',
        text: `The chart runs controller-eligible nodes (\`controller\`, brokers too unless \`controller.controllerOnly\`) and optional broker-only nodes (\`broker\`): give both the same values. \`JMX_PORT\` makes Kafka’s start script open JMX on that port without authentication; the chart’s \`metrics.jmx.enabled\` would add a Prometheus exporter instead, whose names the dashboards don’t read. ${JMX_AGENT}`,
        filename: 'values.yaml (bitnami/kafka)',
        code: ['controller', 'broker']
          .map((role) => `${role}:\n  extraEnvVars:\n    - name: JMX_PORT\n      value: "9999"\n  podAnnotations:\n${indent(checkAnnotation('kafka', 'kafka', jmxCheck(9999)), 4)}`)
          .join('\n'),
      },
      {
        label: 'KRaft with the apache/kafka image (your own StatefulSet)',
        text: `Set \`KAFKA_JMX_PORT\` on the container: the image then opens JMX on it without authentication, reachable at the pod’s IP (\`KAFKA_JMX_HOSTNAME\` defaults to it). Annotate the pod template. ObserveCraft’s own chart does both for its bundled Kafka with \`kafka.jmx.enabled: true\`. ${JMX_AGENT}`,
        filename: 'StatefulSet (pod template)',
        code: `spec:
  template:
    metadata:
      annotations:
${indent(checkAnnotation('kafka', 'kafka', jmxCheck(9999)), 8)}
    spec:
      containers:
        - name: kafka
          image: apache/kafka:4.1.0
          env:
            - name: KAFKA_JMX_PORT
              value: "9999"
          ports:
            - { name: jmx, containerPort: 9999 }`,
      },
    ],
  },
  {
    check: 'kafka_consumer',
    variants: [
      {
        label: 'A cluster check (Datadog chart)',
        text: 'Consumer lag is read from the brokers once per Kafka cluster, so run the check as a cluster check: the Cluster Agent hands it to one Agent, instead of every broker pod’s Agent reporting the same lag. Point `kafka_connect_str` at the cluster’s bootstrap service: Strimzi’s `<cluster>-kafka-bootstrap` (its plain listener on 9092), the Bitnami chart’s `<release>-kafka` (SASL by default: add `security_protocol: SASL_PLAINTEXT`, `sasl_mechanism: PLAIN`, `sasl_plain_username` and `sasl_plain_password`, or set `listeners.client.protocol: PLAINTEXT`), or your own Service. With the Datadog Operator, put the same file in `override.clusterAgent.extraConfd.configDataMap`.',
        filename: 'values.yaml (Datadog chart)',
        code: clusterCheck('kafka_consumer', [{ kafka_connect_str: 'my-cluster-kafka-bootstrap.kafka.svc:9092', monitor_unlisted_consumer_groups: true, tags: ['kafka_cluster:my-cluster'] }]),
      },
    ],
  },
  {
    check: 'strimzi',
    variants: [
      {
        label: 'Strimzi operator Helm chart (strimzi/strimzi-kafka-operator)',
        text: 'The chart’s `annotations` value goes on the Cluster Operator’s pod (its container is `strimzi-cluster-operator`, metrics on 8080). The Kafka pods themselves are the Kafka Broker integration’s, through JMX.',
        filename: 'values.yaml (strimzi-kafka-operator)',
        code: `annotations:\n${indent(checkAnnotation('strimzi-cluster-operator', 'strimzi', { instances: [{ cluster_operator_endpoint: 'http://%%host%%:8080/metrics' }] }), 2)}`,
      },
      {
        label: 'Topic and User Operators (the Kafka resource’s entityOperator)',
        text: 'Both run in the entity operator’s pod, as containers `topic-operator` (metrics on 8080) and `user-operator` (8081): annotate it through `entityOperator.template.pod`.',
        filename: 'Kafka',
        code: `apiVersion: kafka.strimzi.io/v1
kind: Kafka
metadata:
  name: my-cluster
spec:
  entityOperator:
    topicOperator: {}
    userOperator: {}
    template:
      pod:
        metadata:
          annotations:
${indent(checkAnnotation('topic-operator', 'strimzi', { instances: [{ topic_operator_endpoint: 'http://%%host%%:8080/metrics' }] }), 12)}
${indent(checkAnnotation('user-operator', 'strimzi', { instances: [{ user_operator_endpoint: 'http://%%host%%:8081/metrics' }] }), 12)}`,
      },
    ],
  },
  {
    check: 'confluent_platform',
    variants: [
      {
        label: 'Confluent for Kubernetes (Kafka, SchemaRegistry, Connect, … resources)',
        text: `Confluent for Kubernetes opens JMX on port 7203 of every component’s pods. Annotate each component through its \`podTemplate.annotations\`; the container is named after the component (\`kafka\`, \`schemaregistry\`, \`connect\`, \`ksqldb\`, \`kafkarestproxy\`). ${JMX_AGENT}`,
        filename: 'Kafka (platform.confluent.io)',
        code: `apiVersion: platform.confluent.io/v1beta1
kind: Kafka
metadata:
  name: kafka
spec:
  # replicas, image, dataVolumeCapacity, … as before
  podTemplate:
    annotations:
${indent(checkAnnotation('kafka', 'confluent_platform', jmxCheck(7203, { service_check_prefix: 'confluent' })), 6)}`,
      },
      {
        label: 'Confluent images (confluentinc/cp-kafka, cp-schema-registry, …) in your own manifests',
        text: `Set \`KAFKA_JMX_PORT\` (Schema Registry: \`SCHEMA_REGISTRY_JMX_PORT\`, Connect: \`KAFKA_JMX_PORT\`) and \`KAFKA_JMX_HOSTNAME\` to the pod’s IP, so the Agent on the node can follow the JMX connection. ${JMX_AGENT}`,
        filename: 'StatefulSet (pod template)',
        code: `spec:
  template:
    metadata:
      annotations:
${indent(checkAnnotation('kafka', 'confluent_platform', jmxCheck(9999, { service_check_prefix: 'confluent' })), 8)}
    spec:
      containers:
        - name: kafka
          image: confluentinc/cp-kafka
          env:
            - name: KAFKA_JMX_PORT
              value: "9999"
            - name: KAFKA_JMX_HOSTNAME
              valueFrom: { fieldRef: { fieldPath: status.podIP } }`,
      },
    ],
  },
  {
    check: 'postgres',
    variants: [
      {
        label: 'Bitnami PostgreSQL chart (bitnami/postgresql)',
        text: `Put the annotations in the chart’s \`primary.podAnnotations\` (and \`readReplicas.podAnnotations\`); the container is \`postgresql\`. Create the check’s user first (\`CREATE USER datadog WITH PASSWORD '…'; GRANT pg_monitor TO datadog;\`) and keep its password in a secret, here \`datadog-postgres\` in namespace \`db\`. For the log pipeline to read the server’s lines, set \`log_line_prefix = '%m [%p] %d %a %u %h %c '\` in \`primary.extendedConfiguration\`. ${SECRET_NOTE}`,
        filename: POD_ANNOTATIONS,
        code: [
          checkAnnotation('postgresql', 'postgres', {
            instances: [
              {
                host: '%%host%%',
                port: 5432,
                username: 'datadog',
                password: 'ENC[k8s_secret@db/datadog-postgres/password]',
                dbname: 'postgres',
                database_autodiscovery: { enabled: true },
                relations: [{ relation_regex: '.*' }],
              },
            ],
          }),
          logsAnnotation('postgresql', 'postgresql', 'orders-db'),
        ].join('\n'),
      },
      {
        label: 'CloudNativePG (Cluster resource)',
        text: [
          'CloudNativePG copies `spec.inheritedMetadata.annotations` of the `Cluster` to its instance pods, whose container is `postgres`. Let the operator create the check’s user from a basic-auth secret (here `datadog-postgres`, keys `username` and `password`):',
          '```yaml\nspec:\n  managed:\n    roles:\n      - name: datadog\n        ensure: present\n        login: true\n        inRoles: [pg_monitor]\n        passwordSecret:\n          name: datadog-postgres\n```',
          `CloudNativePG writes PostgreSQL’s log as JSON lines (\`logger: postgres\`), which arrive parsed as attributes rather than through the PostgreSQL pipeline. ${SECRET_NOTE}`,
        ].join('\n\n'),
        filename: POD_ANNOTATIONS,
        code: [
          checkAnnotation('postgres', 'postgres', {
            instances: [{ host: '%%host%%', port: 5432, username: 'datadog', password: 'ENC[k8s_secret@db/datadog-postgres/password]', dbname: 'postgres', database_autodiscovery: { enabled: true } }],
          }),
          logsAnnotation('postgres', 'postgresql', 'orders-db'),
        ].join('\n'),
      },
      {
        label: 'Azure Database for PostgreSQL flexible server (managed identity)',
        text: [
          'The server has no pod to annotate: the cluster checks runners run the check, signing in with Microsoft Entra ID through AKS workload identity. Create a user-assigned managed identity, federate it with the runners’ service account (`system:serviceaccount:<namespace>:<release>-cluster-checks` with `rbac.dedicated: true`), make it a Microsoft Entra administrator of the server or create its role as one and grant it `pg_monitor`:',
          '```sql\nSELECT * FROM pgaadauth_create_principal(\'datadog-postgres\', false, false);\nGRANT pg_monitor TO "datadog-postgres";\n```',
          'The check’s `username` is the identity’s name and `client_id` its client ID. `azure.deployment_type` and `fully_qualified_domain_name` name the server as Datadog does; the server’s Azure Monitor metrics come from the Azure integration.',
        ].join('\n\n'),
        filename: CLUSTER_CHECK_VALUES,
        code: clusterCheckValues(
          'postgres',
          {
            host: 'orders-db.postgres.database.azure.com',
            port: 5432,
            username: 'datadog-postgres',
            dbname: 'postgres',
            ssl: 'require',
            azure: {
              deployment_type: 'flexible_server',
              fully_qualified_domain_name: 'orders-db.postgres.database.azure.com',
              managed_authentication: { enabled: true, auth_type: 'workload_identity', client_id: '00000000-0000-0000-0000-000000000000' },
            },
          },
          '  additionalLabels:\n    azure.workload.identity/use: "true"\n  rbac:\n    dedicated: true\n    serviceAccountAnnotations:\n      azure.workload.identity/client-id: 00000000-0000-0000-0000-000000000000\n',
        ),
      },
      {
        label: 'Amazon RDS or Aurora PostgreSQL (IAM authentication)',
        text: [
          'The cluster checks runners run the check with an IAM token instead of a password: turn IAM database authentication on for the instance, give the runners’ service account a role (EKS IAM roles for service accounts) allowed `rds-db:connect` on `dbuser:<resource id>/datadog`, and create the user:',
          '```sql\nCREATE USER datadog;\nGRANT rds_iam TO datadog;\nGRANT pg_monitor TO datadog;\n```',
          'With a password instead, leave `aws.managed_authentication` out and set `password`. The instance’s CloudWatch metrics come from the AWS integration.',
        ].join('\n\n'),
        filename: CLUSTER_CHECK_VALUES,
        code: clusterCheckValues(
          'postgres',
          {
            host: 'orders-db.abcdefghij12.eu-west-1.rds.amazonaws.com',
            port: 5432,
            username: 'datadog',
            dbname: 'postgres',
            ssl: 'require',
            aws: { instance_endpoint: 'orders-db.abcdefghij12.eu-west-1.rds.amazonaws.com', region: 'eu-west-1', managed_authentication: { enabled: true } },
          },
          '  rbac:\n    dedicated: true\n    serviceAccountAnnotations:\n      eks.amazonaws.com/role-arn: arn:aws:iam::123456789012:role/datadog-rds-connect\n',
        ),
      },
    ],
  },
  {
    check: 'pgbouncer',
    variants: [
      {
        label: 'PgBouncer Deployment (bitnami/pgbouncer, edoburu/pgbouncer or your own image)',
        text: `Annotate the pod template of the PgBouncer Deployment (its container here \`pgbouncer\`). The check reads PgBouncer’s admin console, the \`pgbouncer\` database: add the check’s user to \`stats_users\` (\`PGBOUNCER_STATS_USERS=datadog\` with the Bitnami image) and to the auth file. CloudNativePG’s \`Pooler\` only admits certificate logins to the console, which the check can’t make. ${SECRET_NOTE}`,
        filename: POD_ANNOTATIONS,
        code: [
          checkAnnotation('pgbouncer', 'pgbouncer', { instances: [{ database_url: 'postgresql://datadog:ENC[k8s_secret@db/datadog-pgbouncer/password]@%%host%%:6432/pgbouncer' }] }),
          logsAnnotation('pgbouncer', 'pgbouncer', 'pgbouncer'),
        ].join('\n'),
      },
    ],
  },
  {
    check: 'redisdb',
    variants: [
      {
        label: 'Bitnami Redis chart (bitnami/redis)',
        text: `Put the annotations in \`master.podAnnotations\` and \`replica.podAnnotations\`; the container is \`redis\` and the chart keeps the password in the secret \`<release>-redis\` (key \`redis-password\`), here release \`cache\` in namespace \`cache\`. ${SECRET_NOTE}`,
        filename: POD_ANNOTATIONS,
        code: [
          checkAnnotation('redis', 'redisdb', { instances: [{ host: '%%host%%', port: 6379, password: 'ENC[k8s_secret@cache/cache-redis/redis-password]' }] }),
          logsAnnotation('redis', 'redis', 'catalog-cache'),
        ].join('\n'),
      },
      {
        label: 'Valkey, Bitnami Valkey chart (bitnami/valkey)',
        text: `Valkey answers the Redis check as Redis does; its metrics keep the \`redis.*\` names, so the Redis dashboard and monitors apply. Put the annotations in \`primary.podAnnotations\` and \`replica.podAnnotations\`; the container is \`valkey\` and the password is in \`<release>-valkey\` (key \`valkey-password\`). Its log lines have Redis’s format, so the Redis log pipeline (\`source: redis\`) parses them. ${SECRET_NOTE}`,
        filename: POD_ANNOTATIONS,
        code: [
          checkAnnotation('valkey', 'redisdb', { instances: [{ host: '%%host%%', port: 6379, password: 'ENC[k8s_secret@cache/sessions-valkey/valkey-password]', tags: ['engine:valkey'] }] }),
          logsAnnotation('valkey', 'redis', 'session-store'),
        ].join('\n'),
      },
      {
        label: 'Redis or Valkey image in your own StatefulSet',
        text: 'Annotate the pod template with the container’s name (here `redis`). Without a password, leave `password` out.',
        filename: POD_ANNOTATIONS,
        code: [checkAnnotation('redis', 'redisdb', { instances: [{ host: '%%host%%', port: 6379 }] }), logsAnnotation('redis', 'redis', 'rate-limiter')].join('\n'),
      },
    ],
  },
  {
    check: 'clickhouse',
    variants: [
      {
        label: 'Altinity ClickHouse Operator (ClickHouseInstallation)',
        text: [
          'Add the annotations to the pod template the installation uses (`spec.templates.podTemplates[].metadata.annotations`, container `clickhouse`). The check queries the HTTP interface (8123) with a read-only user, defined in the installation:',
          "```yaml\nspec:\n  configuration:\n    users:\n      datadog/k8s_secret_password: clickhouse/datadog-clickhouse/password\n      datadog/networks/ip: ['10.0.0.0/8']\n      datadog/profile: readonly\n```",
          SECRET_NOTE,
        ].join('\n\n'),
        filename: POD_ANNOTATIONS,
        code: [
          checkAnnotation('clickhouse', 'clickhouse', { instances: [{ server: '%%host%%', port: 8123, username: 'datadog', password: 'ENC[k8s_secret@clickhouse/datadog-clickhouse/password]' }] }),
          logsAnnotation('clickhouse', 'clickhouse', 'clickhouse'),
        ].join('\n'),
      },
      {
        label: 'Bitnami ClickHouse chart (bitnami/clickhouse)',
        text: `Put the annotations in the chart’s \`podAnnotations\`; the container is \`clickhouse\`. Here the check uses the chart’s \`default\` user, whose password is in the secret \`<release>-clickhouse\` (key \`admin-password\`); a read-only user of your own is better. ${SECRET_NOTE}`,
        filename: POD_ANNOTATIONS,
        code: [
          checkAnnotation('clickhouse', 'clickhouse', { instances: [{ server: '%%host%%', port: 8123, username: 'default', password: 'ENC[k8s_secret@analytics/events-clickhouse/admin-password]' }] }),
          logsAnnotation('clickhouse', 'clickhouse', 'clickhouse'),
        ].join('\n'),
      },
    ],
  },
  {
    check: 'nginx_ingress_controller',
    variants: [
      {
        label: 'ingress-nginx Helm chart (kubernetes/ingress-nginx)',
        text: 'The controller serves its Prometheus metrics on port 10254, next to its health checks; `controller.metrics.enabled: true` also gives them a Service. Put the annotations in the chart’s `controller.podAnnotations` value (the container is `controller`). The logs annotation names the source the integration’s log pipeline reads; without it the Agent would name them after the image (`controller`).',
        filename: POD_ANNOTATIONS,
        code: [
          checkAnnotation('controller', 'nginx_ingress_controller', { instances: [{ prometheus_url: 'http://%%host%%:10254/metrics' }] }),
          logsAnnotation('controller', 'nginx-ingress-controller', 'ingress-nginx'),
        ].join('\n'),
      },
    ],
  },
  {
    check: 'kube_apiserver_metrics',
    variants: [
      {
        label: 'Managed control plane (AKS, EKS) or any cluster',
        text: 'The API server isn’t a pod you can annotate on a managed cluster, but it answers on the `kubernetes` Service in `default`: run the check as a cluster check from the Cluster Agent (`clusterAgent.enabled` and `datadog.clusterChecks.enabled`, the chart’s defaults). The Agent’s service account already may read `/metrics`. Self-managed control planes with static `kube-apiserver` pods are found by the Agent by itself.',
        filename: CLUSTER_CHECK_VALUES,
        code: endpointsClusterCheck('kube_apiserver_metrics', { name: 'kubernetes', namespace: 'default' }, [{ prometheus_url: 'https://%%host%%:%%port%%/metrics', bearer_token_auth: true }]),
      },
    ],
  },
  ...(
    [
      ['kube_scheduler', 'scheduler', 'ksh', 'kube-scheduler'],
      ['kube_controller_manager', 'controller manager', 'kcm', 'kube-controller-manager'],
    ] as const
  ).map(([check, name, path, pod]) => ({
    check,
    variants: [
      {
        label: 'Amazon EKS',
        text: `EKS serves the ${name}’s metrics through the API server (\`/apis/metrics.eks.amazonaws.com/v1/${path}/container/metrics\`, Kubernetes 1.28 and later): run the check as a cluster check against the \`kubernetes\` Service. Let the Agent read them with a ClusterRole allowing \`get\` on \`${path}/metrics\` in the \`metrics.eks.amazonaws.com\` API group, bound to the Agent’s service account.`,
        filename: CLUSTER_CHECK_VALUES,
        code: endpointsClusterCheck(check, { name: 'kubernetes', namespace: 'default' }, [
          {
            prometheus_url: `https://%%host%%:%%port%%/apis/metrics.eks.amazonaws.com/v1/${path}/container/metrics`,
            extra_headers: { accept: '*/*' },
            bearer_token_auth: true,
            tls_ca_cert: '/var/run/secrets/kubernetes.io/serviceaccount/ca.crt',
          },
        ]),
      },
      {
        label: 'Azure AKS',
        text: `AKS runs the ${name} in Microsoft’s control plane, which exposes no metrics endpoint to the cluster: there is nothing to annotate, and this check can’t run there.`,
      },
      {
        label: 'Self-managed control plane (kubeadm, kOps)',
        text: `The Agent on a control-plane node finds the static \`${pod}\` pod and runs the check by itself; let the Agent run there (a toleration for the control-plane taint).`,
      },
    ],
  })),
];

/** The Kubernetes hint of an Agent check, if there is one. */
export function kubernetesHint(check: string): KubernetesHint | undefined {
  return HINTS.find((h) => h.check === check);
}

/** Every Agent check's Kubernetes hint. */
export function kubernetesHints(): readonly KubernetesHint[] {
  return HINTS;
}
