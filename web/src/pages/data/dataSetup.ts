// How database and data job metrics reach ObserveCraft: the Datadog Agent's integration checks or the
// OpenTelemetry Collector's receivers. {{intake}} and {{apiKey}} are filled in per install.
import type { DbEngine } from '../../api/databases/client';
import { collectorConfig } from '../setup/collector';
import type { SetupOption } from '../setup/SetupOptions';
import type { SetupStep } from '../setup/sources';

export interface EngineSetup {
  engine: DbEngine;
  /** Brand logo key (scripts/build-logos.mjs), when there is one. */
  logo?: string;
  name: string;
  /** Unit of the throughput column for this engine. */
  throughput: string;
  agent: SetupStep[];
  otel: SetupStep[];
}

const restart: SetupStep = { title: 'Restart the Agent', text: 'The instance appears in Database Monitoring within a minute or two.' };
const agentFirst: SetupStep = {
  title: 'Run the Datadog Agent next to the database',
  text: 'On the database host or as a sidecar / cluster check, already sending to ObserveCraft (Infrastructure → Add Resources).',
};
const otelFirst: SetupStep = {
  title: 'Run an OpenTelemetry Collector that can reach the database',
  text: 'Any Collector works (the observecraft-agent chart, or one on the database host); add the receiver below to its configuration.',
};

export const ENGINES: EngineSetup[] = [
  {
    engine: 'postgres',
    logo: 'postgresql',
    name: 'PostgreSQL',
    throughput: 'transactions/s',
    agent: [
      agentFirst,
      { title: 'Create a monitoring user', code: "CREATE USER datadog WITH PASSWORD '<PASSWORD>';\nGRANT pg_monitor TO datadog;" },
      {
        title: 'Configure the check (conf.d/postgres.d/conf.yaml)',
        code: 'instances:\n  - host: localhost\n    port: 5432\n    username: datadog\n    password: <PASSWORD>\n    tags: [env:prod, service:orders-db]',
      },
      restart,
    ],
    otel: [
      otelFirst,
      { title: 'Create a monitoring user', code: "CREATE USER otel WITH PASSWORD '<PASSWORD>';\nGRANT pg_monitor TO otel;" },
      {
        title: 'Add the postgresql receiver',
        code: collectorConfig('postgresql', '  postgresql:\n    endpoint: localhost:5432\n    username: otel\n    password: <PASSWORD>\n    collection_interval: 30s\n    tls:\n      insecure: true'),
      },
    ],
  },
  {
    engine: 'mysql',
    logo: 'mysql',
    name: 'MySQL',
    throughput: 'queries/s',
    agent: [
      agentFirst,
      {
        title: 'Create a monitoring user',
        code: "CREATE USER 'datadog'@'%' IDENTIFIED BY '<PASSWORD>';\nGRANT REPLICATION CLIENT, PROCESS ON *.* TO 'datadog'@'%';\nGRANT SELECT ON performance_schema.* TO 'datadog'@'%';",
      },
      {
        title: 'Configure the check (conf.d/mysql.d/conf.yaml)',
        code: 'instances:\n  - host: localhost\n    port: 3306\n    username: datadog\n    password: <PASSWORD>\n    options:\n      replication: true',
      },
      restart,
    ],
    otel: [
      otelFirst,
      { title: 'Create a monitoring user', code: "CREATE USER 'otel'@'%' IDENTIFIED BY '<PASSWORD>';\nGRANT PROCESS, REPLICATION CLIENT ON *.* TO 'otel'@'%';" },
      { title: 'Add the mysql receiver', code: collectorConfig('mysql', '  mysql:\n    endpoint: localhost:3306\n    username: otel\n    password: <PASSWORD>\n    collection_interval: 30s') },
    ],
  },
  {
    engine: 'sqlserver',
    name: 'SQL Server',
    throughput: 'batches/s',
    agent: [
      agentFirst,
      { title: 'Create a monitoring login', code: "CREATE LOGIN datadog WITH PASSWORD = '<PASSWORD>';\nCREATE USER datadog FOR LOGIN datadog;\nGRANT VIEW SERVER STATE TO datadog;" },
      { title: 'Configure the check (conf.d/sqlserver.d/conf.yaml)', code: 'instances:\n  - host: localhost,1433\n    username: datadog\n    password: <PASSWORD>\n    connector: odbc' },
      restart,
    ],
    otel: [
      otelFirst,
      {
        title: 'Add the sqlserver receiver',
        text: 'On Windows it reads the performance counters; elsewhere give it a connection.',
        code: collectorConfig('sqlserver', '  sqlserver:\n    server: localhost\n    port: 1433\n    username: otel\n    password: <PASSWORD>\n    collection_interval: 30s'),
      },
    ],
  },
  {
    engine: 'mongodb',
    logo: 'mongodb',
    name: 'MongoDB',
    throughput: 'operations/s',
    agent: [
      agentFirst,
      { title: 'Create a monitoring user', code: 'db.getSiblingDB("admin").createUser({ user: "datadog", pwd: "<PASSWORD>", roles: [{ role: "clusterMonitor", db: "admin" }] })' },
      { title: 'Configure the check (conf.d/mongo.d/conf.yaml)', code: 'instances:\n  - hosts: [localhost:27017]\n    username: datadog\n    password: <PASSWORD>\n    database: admin' },
      restart,
    ],
    otel: [
      otelFirst,
      {
        title: 'Add the mongodb receiver',
        code: collectorConfig(
          'mongodb',
          '  mongodb:\n    hosts:\n      - endpoint: localhost:27017\n    username: otel\n    password: <PASSWORD>\n    collection_interval: 30s\n    tls:\n      insecure: true',
        ),
      },
    ],
  },
  {
    engine: 'redis',
    logo: 'redis',
    name: 'Redis',
    throughput: 'commands/s',
    agent: [agentFirst, { title: 'Configure the check (conf.d/redisdb.d/conf.yaml)', code: 'instances:\n  - host: localhost\n    port: 6379\n    # password: <PASSWORD>' }, restart],
    otel: [otelFirst, { title: 'Add the redis receiver', code: collectorConfig('redis', '  redis:\n    endpoint: localhost:6379\n    collection_interval: 30s\n    # password: <PASSWORD>') }],
  },
];

export const engineSetup = (engine: DbEngine): EngineSetup => ENGINES.find((e) => e.engine === engine) ?? ENGINES[0];

/** The two ways to monitor an engine, as setup options. */
export function engineOptions(e: EngineSetup): SetupOption[] {
  return [
    { id: 'agent', name: 'Datadog Agent', summary: `The Agent's ${e.name} check reports connections, throughput, cache, replication and size metrics.`, steps: e.agent },
    { id: 'otel', name: 'OpenTelemetry Collector', summary: `The Collector's ${e.name} receiver reports the same kind of metrics over OTLP.`, steps: e.otel },
  ];
}

export const JOB_SETUP: SetupOption[] = [
  {
    id: 'airflow',
    name: 'Airflow',
    summary: 'Airflow sends StatsD metrics (DAG run durations, task successes and failures) to the Datadog Agent, which forwards them tagged with the DAG.',
    steps: [
      { title: 'Send Airflow metrics to the Agent (airflow.cfg)', code: '[metrics]\nstatsd_on = True\nstatsd_host = localhost\nstatsd_port = 8125\nstatsd_prefix = airflow' },
      {
        title: 'Map metric names to tags (datadog.yaml)',
        text: "Airflow puts the DAG id in the metric name; the Agent's StatsD mapper turns it into a dag_id tag. Map at least the DAG run durations and task counters.",
        code: 'dogstatsd_mapper_profiles:\n  - name: airflow\n    prefix: "airflow."\n    mappings:\n      - match: "airflow.dagrun.duration.success.*"\n        name: "airflow.dagrun.duration.success"\n        tags:\n          dag_id: "$1"\n      - match: "airflow.dagrun.duration.failed.*"\n        name: "airflow.dagrun.duration.failed"\n        tags:\n          dag_id: "$1"\n      - match: "airflow.ti_failures.*.*"\n        name: "airflow.ti_failures"\n        tags:\n          dag_id: "$1"\n          task_id: "$2"',
      },
      { title: 'Restart Airflow and the Agent', text: 'DAGs appear here once they have run.' },
    ],
  },
  {
    id: 'spark',
    name: 'Spark',
    summary: "The Agent's Spark check reads the Spark REST API (driver, standalone master, YARN or Mesos) and reports job, stage and executor counts per application.",
    steps: [
      { title: 'Configure the check (conf.d/spark.d/conf.yaml)', code: 'instances:\n  - spark_url: http://localhost:4040\n    spark_cluster_mode: spark_driver_mode\n    cluster_name: shop-spark' },
      { title: 'Restart the Agent', text: 'Applications appear here with their succeeded, failed and running jobs.' },
    ],
  },
  {
    id: 'databricks',
    name: 'Databricks',
    summary: "Databricks runs Spark: install the Datadog Agent on the cluster's driver with an init script and point its Spark check at the driver.",
    steps: [
      {
        title: 'Add a cluster init script',
        text: "Install the Agent on the driver node (Databricks' init scripts run at cluster start), set dd_url to this install and enable the Spark check with spark_cluster_mode: spark_driver_mode.",
      },
      { title: 'Check the data', text: 'Each Databricks job run shows up as a Spark application, tagged with its cluster.' },
    ],
  },
];
