// How to run an Agent check against a workload on Kubernetes: the Datadog Agent's Autodiscovery annotations on the
// workload's pods (`ad.datadoghq.com/<container>.checks`), per check, for the ways the software is usually deployed;
// for managed databases outside the cluster (Azure Database for PostgreSQL, RDS), a cluster check the Datadog Helm
// chart's cluster checks runners run. Our own words and examples; shown on the integration's Configure tab below the
// Agent configuration form.

export interface KubernetesHintVariant {
  /** How the software is deployed (its Helm chart, the Omnibus image). */
  label: string;
  /** Where the annotations go and what to set on the software's side. */
  text: string;
  /** The pod annotations (YAML), or */
  annotations?: string;
  /** the Datadog Helm chart's values running it as a cluster check (YAML). */
  values?: string;
}

export interface KubernetesHint {
  check: string;
  variants: KubernetesHintVariant[];
}

/** `ad.datadoghq.com/<container>.checks` with the check's configuration, as pod annotations in YAML. */
export function checkAnnotation(container: string, check: string, config: { init_config?: object; instances: object[] }): string {
  const body = JSON.stringify({ [check]: { init_config: config.init_config ?? {}, instances: config.instances } }, null, 2);
  return `ad.datadoghq.com/${container}.checks: |\n${body
    .split('\n')
    .map((l) => `  ${l}`)
    .join('\n')}`;
}

/** `ad.datadoghq.com/<container>.logs`: the container's logs tagged with the integration's source (its pipeline's filter). */
export function logsAnnotation(container: string, source: string, service: string): string {
  return `ad.datadoghq.com/${container}.logs: '${JSON.stringify([{ source, service }])}'`;
}

/** The Datadog Helm chart's values running a check on the cluster checks runners (`clusterAgent.confd`), plus `extra` values. */
export function clusterCheckValues(check: string, instance: object, extra = ''): string {
  const body = JSON.stringify({ cluster_check: true, init_config: {}, instances: [instance] }, null, 2);
  const indented = body
    .split('\n')
    .map((l) => `      ${l}`)
    .join('\n');
  return `clusterChecksRunner:\n  enabled: true\n${extra}clusterAgent:\n  confd:\n    ${check}.yaml: |\n${indented}`;
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
        annotations: checkAnnotation('webservice', 'gitlab', {
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
        annotations: checkAnnotation('gitlab', 'gitlab', {
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
        annotations: checkAnnotation('gitlab-runner', 'gitlab_runner', {
          init_config: { allowed_metrics: RUNNER_METRICS },
          instances: [{ prometheus_endpoint: 'http://%%host%%:9252/metrics', gitlab_url: 'https://gitlab.example.com' }],
        }),
      },
    ],
  },
  {
    check: 'postgres',
    variants: [
      {
        label: 'Bitnami PostgreSQL chart (bitnami/postgresql)',
        text: `Put the annotations in the chart’s \`primary.podAnnotations\` (and \`readReplicas.podAnnotations\`); the container is \`postgresql\`. Create the check’s user first (\`CREATE USER datadog WITH PASSWORD '…'; GRANT pg_monitor TO datadog;\`) and keep its password in a secret, here \`datadog-postgres\` in namespace \`db\`. For the log pipeline to read the server’s lines, set \`log_line_prefix = '%m [%p] %d %a %u %h %c '\` in \`primary.extendedConfiguration\`. ${SECRET_NOTE}`,
        annotations: [
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
        annotations: [
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
        values: clusterCheckValues(
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
        values: clusterCheckValues(
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
        annotations: [
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
        annotations: [
          checkAnnotation('redis', 'redisdb', { instances: [{ host: '%%host%%', port: 6379, password: 'ENC[k8s_secret@cache/cache-redis/redis-password]' }] }),
          logsAnnotation('redis', 'redis', 'catalog-cache'),
        ].join('\n'),
      },
      {
        label: 'Valkey, Bitnami Valkey chart (bitnami/valkey)',
        text: `Valkey answers the Redis check as Redis does; its metrics keep the \`redis.*\` names, so the Redis dashboard and monitors apply. Put the annotations in \`primary.podAnnotations\` and \`replica.podAnnotations\`; the container is \`valkey\` and the password is in \`<release>-valkey\` (key \`valkey-password\`). Its log lines have Redis’s format, so the Redis log pipeline (\`source: redis\`) parses them. ${SECRET_NOTE}`,
        annotations: [
          checkAnnotation('valkey', 'redisdb', { instances: [{ host: '%%host%%', port: 6379, password: 'ENC[k8s_secret@cache/sessions-valkey/valkey-password]', tags: ['engine:valkey'] }] }),
          logsAnnotation('valkey', 'redis', 'session-store'),
        ].join('\n'),
      },
      {
        label: 'Redis or Valkey image in your own StatefulSet',
        text: 'Annotate the pod template with the container’s name (here `redis`). Without a password, leave `password` out.',
        annotations: [checkAnnotation('redis', 'redisdb', { instances: [{ host: '%%host%%', port: 6379 }] }), logsAnnotation('redis', 'redis', 'rate-limiter')].join('\n'),
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
        annotations: [
          checkAnnotation('clickhouse', 'clickhouse', { instances: [{ server: '%%host%%', port: 8123, username: 'datadog', password: 'ENC[k8s_secret@clickhouse/datadog-clickhouse/password]' }] }),
          logsAnnotation('clickhouse', 'clickhouse', 'clickhouse'),
        ].join('\n'),
      },
      {
        label: 'Bitnami ClickHouse chart (bitnami/clickhouse)',
        text: `Put the annotations in the chart’s \`podAnnotations\`; the container is \`clickhouse\`. Here the check uses the chart’s \`default\` user, whose password is in the secret \`<release>-clickhouse\` (key \`admin-password\`); a read-only user of your own is better. ${SECRET_NOTE}`,
        annotations: [
          checkAnnotation('clickhouse', 'clickhouse', { instances: [{ server: '%%host%%', port: 8123, username: 'default', password: 'ENC[k8s_secret@analytics/events-clickhouse/admin-password]' }] }),
          logsAnnotation('clickhouse', 'clickhouse', 'clickhouse'),
        ].join('\n'),
      },
    ],
  },
];

/** The Kubernetes hint of an Agent check, if there is one. */
export function kubernetesHint(check: string): KubernetesHint | undefined {
  return HINTS.find((h) => h.check === check);
}
