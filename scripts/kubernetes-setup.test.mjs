import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';
import { datadogValues, helmCommands, intakeEndpoints, otelValues } from '../web/src/pages/integrations/kubernetes/kubernetesSetup.ts';

const require = createRequire(new URL('../web/package.json', import.meta.url));
const yaml = require('js-yaml');

const BASE = { clusterName: 'shop-prod', intakeUrl: 'https://intake.observecraft.example.com', logs: true, apm: true, autoscaling: false };

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
