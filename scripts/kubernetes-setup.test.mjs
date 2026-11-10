import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { EXAMPLE_OPTIONS, datadogOperatorManifest, datadogValues, helmCommands, intakeEndpoints, otelValues } from '../web/src/pages/integrations/kubernetes/kubernetesSetup.ts';
import { exampleFiles } from './datadog-examples.mjs';

const require = createRequire(new URL('../web/package.json', import.meta.url));
const yaml = require('js-yaml');

const BASE = {
  clusterName: 'shop-prod',
  intakeUrl: 'https://intake.observecraft.example.com',
  logs: true,
  apm: true,
  autoscaling: false,
  containers: true,
  processes: false,
  sbom: true,
  workloadProtection: true,
  networkMonitoring: false,
};
const envOf = (list) => Object.fromEntries(list.map((e) => [e.name, e.value]));

test('intake endpoints come from one URL: the logs intake is host:port and keeps TLS', () => {
  assert.deepEqual(intakeEndpoints('https://intake.observecraft.example.com/'), {
    url: 'https://intake.observecraft.example.com',
    logs: 'intake.observecraft.example.com:443',
    profiles: 'https://intake.observecraft.example.com/api/v2/profile',
    tls: true,
  });
  assert.deepEqual(intakeEndpoints('http://observecraft-intake.observecraft.svc:4318'), {
    url: 'http://observecraft-intake.observecraft.svc:4318',
    logs: 'observecraft-intake.observecraft.svc:4318',
    profiles: 'http://observecraft-intake.observecraft.svc:4318/api/v2/profile',
    tls: false,
  });
});

test('the Datadog chart values send everything to the intake and nothing to Datadog', () => {
  const values = yaml.load(datadogValues(BASE));
  assert.equal(values.datadog.clusterName, 'shop-prod');
  assert.equal(values.datadog.site, 'observecraft.invalid');
  assert.equal(values.datadog.dd_url, 'https://intake.observecraft.example.com');
  assert.equal(values.datadog.apiKeyExistingSecret, 'datadog-secret');
  assert.equal(values.datadog.orchestratorExplorer.enabled, true);
  assert.equal(values.datadog.logs.enabled, true);
  assert.equal(values.datadog.apm.portEnabled, true);
  const env = Object.fromEntries(values.datadog.env.map((e) => [e.name, e.value]));
  assert.equal(env.DD_ORCHESTRATOR_EXPLORER_ORCHESTRATOR_DD_URL, 'https://intake.observecraft.example.com');
  assert.equal(env.DD_LOGS_CONFIG_LOGS_DD_URL, 'intake.observecraft.example.com:443');
  assert.equal(env.DD_LOGS_CONFIG_LOGS_NO_SSL, undefined, 'TLS intakes keep SSL');
  assert.deepEqual(values.clusterAgent.env, values.datadog.env, 'the Cluster Agent sends to the same intake');
  // Without autoscaling, Remote Configuration stays off.
  assert.equal(values.datadog.remoteConfiguration.enabled, false);
  assert.equal(values.datadog.autoscaling, undefined);
});

test('a plain-HTTP intake turns off SSL for logs; logs and APM can be left out', () => {
  const values = yaml.load(datadogValues({ ...BASE, intakeUrl: 'http://observecraft-intake.observecraft.svc:4318', logs: false, apm: false }));
  const env = Object.fromEntries(values.datadog.env.map((e) => [e.name, e.value]));
  assert.equal(env.DD_LOGS_CONFIG_LOGS_NO_SSL, 'true');
  assert.equal(values.datadog.logs.enabled, false);
  assert.equal(values.datadog.apm.portEnabled, false);
});

test('workload autoscaling enables it, with Remote Configuration (the Cluster Agent requires it) and the admission controller', () => {
  const values = yaml.load(datadogValues({ ...BASE, autoscaling: true }));
  assert.equal(values.datadog.autoscaling.workload.enabled, true);
  assert.equal(values.datadog.remoteConfiguration.enabled, true);
  assert.equal(values.clusterAgent.admissionController.enabled, true);
  // The external recommender (ObserveCraft) is off by default in the Cluster Agent.
  const clusterEnv = Object.fromEntries(values.clusterAgent.env.map((e) => [e.name, e.value]));
  assert.equal(clusterEnv.DD_AUTOSCALING_WORKLOAD_EXTERNAL_RECOMMENDER_ENABLED, 'true');
  assert.equal(clusterEnv.DD_ORCHESTRATOR_EXPLORER_ORCHESTRATOR_DD_URL, 'https://intake.observecraft.example.com');
  // Still nothing reaches Datadog: the site is not a Datadog site.
  assert.equal(values.datadog.site, 'observecraft.invalid');
});

test('the observecraft-agent values carry the endpoint, cluster and log choice', () => {
  const values = yaml.load(otelValues({ ...BASE, logs: false }));
  assert.equal(values.endpoint, 'https://intake.observecraft.example.com');
  assert.equal(values.clusterName, 'shop-prod');
  assert.equal(values.existingSecret, 'observecraft-agent');
  assert.equal(values.node.logs.enabled, false);
});

test('the commands create the key secret, then install the chart with the values file', () => {
  assert.deepEqual(helmCommands('datadog'), [
    'kubectl create namespace datadog',
    'kubectl -n datadog create secret generic datadog-secret --from-literal api-key=<INGEST_API_KEY>',
    'helm repo add datadog https://helm.datadoghq.com && helm repo update',
    'helm install datadog datadog/datadog -n datadog -f values.yaml',
  ]);
  assert.deepEqual(helmCommands('opentelemetry'), [
    'kubectl create namespace observecraft',
    'kubectl -n observecraft create secret generic observecraft-agent --from-literal api-key=<INGEST_API_KEY>',
    'helm install observecraft-agent oci://ghcr.io/observecraft/charts/observecraft-agent -n observecraft -f values.yaml',
  ]);
});

test('live containers are collected (the intake takes /api/v1/container); live processes are a choice', () => {
  const on = yaml.load(datadogValues(BASE));
  assert.equal(on.datadog.processAgent.containerCollection, true);
  assert.equal(on.datadog.processAgent.processCollection, false);
  assert.equal(envOf(on.datadog.env).DD_PROCESS_CONFIG_CONTAINER_COLLECTION_ENABLED, undefined, 'not forced off');
  assert.equal(envOf(on.datadog.env).DD_PROCESS_CONFIG_PROCESS_DD_URL, 'https://intake.observecraft.example.com');
  const off = yaml.load(datadogValues({ ...BASE, containers: false, processes: true }));
  assert.equal(off.datadog.processAgent.containerCollection, false);
  assert.equal(off.datadog.processAgent.processCollection, true);
});

test('SBOMs, Workload Protection and Cloud Network Monitoring are options', () => {
  const on = yaml.load(datadogValues({ ...BASE, networkMonitoring: true }));
  const env = envOf(on.datadog.env);
  assert.equal(env.DD_SBOM_HOST_ENABLED, 'true');
  assert.equal(env.DD_SBOM_CONTAINER_IMAGE_ENABLED, 'true');
  assert.equal(env.DD_SBOM_DD_URL, 'https://intake.observecraft.example.com');
  assert.equal(on.datadog.securityAgent.runtime.enabled, true);
  assert.equal(env.DD_RUNTIME_SECURITY_CONFIG_ENDPOINTS_LOGS_DD_URL, 'intake.observecraft.example.com:443');
  assert.equal(on.datadog.networkMonitoring.enabled, true);
  const off = yaml.load(datadogValues({ ...BASE, sbom: false, workloadProtection: false }));
  const offEnv = envOf(off.datadog.env);
  assert.equal(offEnv.DD_SBOM_ENABLED, undefined);
  assert.equal(offEnv.DD_RUNTIME_SECURITY_CONFIG_ENDPOINTS_LOGS_DD_URL, undefined);
  assert.equal(off.datadog.securityAgent.runtime.enabled, false);
  assert.equal(off.datadog.networkMonitoring.enabled, false);
});

test('the Datadog Operator gets the same choices as a DatadogAgent', () => {
  const agent = yaml.load(datadogOperatorManifest({ ...BASE, networkMonitoring: true, processes: true }));
  assert.equal(agent.kind, 'DatadogAgent');
  assert.equal(agent.spec.global.site, 'observecraft.invalid');
  assert.equal(agent.spec.global.clusterName, 'shop-prod');
  assert.equal(agent.spec.global.endpoint.url, 'https://intake.observecraft.example.com');
  const f = agent.spec.features;
  assert.equal(f.liveContainerCollection.enabled, true);
  assert.equal(f.liveProcessCollection.enabled, true);
  assert.equal(f.npm.enabled, true);
  assert.equal(f.cws.enabled, true);
  assert.equal(f.logCollection.enabled, true);
  assert.equal(f.remoteConfiguration.enabled, false);
  assert.deepEqual(envOf(agent.spec.override.nodeAgent.env), envOf(yaml.load(datadogValues(BASE)).datadog.env), 'the same env as the Helm values');
  assert.deepEqual(agent.spec.override.clusterAgent.env, agent.spec.override.nodeAgent.env);
  const scaling = yaml.load(datadogOperatorManifest({ ...BASE, autoscaling: true }));
  assert.equal(scaling.spec.features.autoscaling.workload.enabled, true);
  assert.equal(scaling.spec.features.remoteConfiguration.enabled, true);
  assert.equal(scaling.spec.features.admissionController.enabled, true);
  assert.equal(envOf(scaling.spec.override.clusterAgent.env).DD_AUTOSCALING_WORKLOAD_EXTERNAL_RECOMMENDER_ENABLED, 'true');
  assert.deepEqual(helmCommands('datadog-operator'), [
    'helm repo add datadog https://helm.datadoghq.com && helm repo update',
    'helm install datadog-operator datadog/datadog-operator -n datadog --create-namespace',
    'kubectl -n datadog create secret generic datadog-secret --from-literal api-key=<INGEST_API_KEY>',
    'kubectl apply -f datadog-agent.yaml',
  ]);
});

test('deploy/examples/datadog is the generator output for the in-cluster intake (node scripts/datadog-examples.mjs)', () => {
  for (const [path, content] of Object.entries(exampleFiles())) {
    assert.equal(readFileSync(new URL(`../${path}`, import.meta.url), 'utf8'), content, path);
  }
  const values = yaml.load(datadogValues(EXAMPLE_OPTIONS));
  assert.equal(values.datadog.processAgent.containerCollection, true);
});
