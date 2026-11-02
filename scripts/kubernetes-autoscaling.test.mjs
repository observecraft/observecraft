import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';
import {
  DEFAULT_PROFILE,
  cores,
  mebibytes,
  podAutoscalerManifest,
  profileManifest,
  profileTemplate,
  recommenderEndpoint,
  setResourcesCommand,
  uninstallCommands,
  labelCommand,
} from '../web/src/pages/kubernetes/autoscaling/manifests.ts';

const require = createRequire(new URL('../web/package.json', import.meta.url));
const yaml = require('js-yaml');

test('quantities are written the Kubernetes way', () => {
  assert.equal(cores(0.231), '231m');
  assert.equal(cores(2), '2');
  assert.equal(cores(0.0004), '1m');
  assert.equal(mebibytes(589 * 1024 * 1024), '589Mi');
  assert.equal(mebibytes(2 * 1024 ** 3), '2048Mi');
});

test('the default profile scales on both dimensions with gentle scale-down', () => {
  const t = profileTemplate(DEFAULT_PROFILE);
  assert.deepEqual(t.applyPolicy, {
    mode: 'Apply',
    scaleUp: { strategy: 'Max', stabilizationWindowSeconds: 600, rules: [{ type: 'Percent', value: 50, periodSeconds: 120 }] },
    scaleDown: { strategy: 'Max', stabilizationWindowSeconds: 600, rules: [{ type: 'Percent', value: 20, periodSeconds: 1200 }] },
    update: { strategy: 'Auto' },
  });
  assert.deepEqual(t.objectives, [{ type: 'PodResource', podResource: { name: 'cpu', value: { type: 'Utilization', utilization: 75 } } }]);
  assert.deepEqual(t.constraints, { minReplicas: 2, maxReplicas: 10, containers: [{ name: '*', enabled: true }] });
});

test('vertical-only and horizontal-only profiles disable the other dimension', () => {
  const vertical = profileTemplate({ ...DEFAULT_PROFILE, dimensions: 'vertical' });
  assert.deepEqual([vertical.applyPolicy.scaleUp, vertical.applyPolicy.scaleDown], [{ strategy: 'Disabled' }, { strategy: 'Disabled' }]);
  assert.equal(vertical.objectives, undefined);
  assert.deepEqual(vertical.constraints, { containers: [{ name: '*', enabled: true }] });
  const horizontal = profileTemplate({ ...DEFAULT_PROFILE, dimensions: 'horizontal', container: 'app' });
  assert.deepEqual(horizontal.applyPolicy.update, { strategy: 'Disabled' });
  assert.deepEqual(horizontal.objectives, [{ type: 'ContainerResource', containerResource: { name: 'cpu', container: 'app', value: { type: 'Utilization', utilization: 75 } } }]);
  assert.equal(horizontal.constraints.containers, undefined);
});

test('the profile manifest is a DatadogPodAutoscalerClusterProfile carrying the template', () => {
  const doc = yaml.load(profileManifest('checkout-balanced', profileTemplate(DEFAULT_PROFILE)));
  assert.equal(doc.apiVersion, 'datadoghq.com/v1alpha2');
  assert.equal(doc.kind, 'DatadogPodAutoscalerClusterProfile');
  assert.deepEqual(doc.metadata, { name: 'checkout-balanced' });
  assert.deepEqual(doc.spec.template, profileTemplate(DEFAULT_PROFILE));
});

test('a workload autoscaler points the Cluster Agent at ObserveCraft’s recommender', () => {
  const endpoint = recommenderEndpoint('https://intake.observecraft.example.com/');
  assert.equal(endpoint, 'https://intake.observecraft.example.com/api/v2/autoscaling/recommend?api_key=<INGEST_API_KEY>');
  const doc = yaml.load(podAutoscalerManifest({ namespace: 'checkout', kind: 'Deployment', name: 'storefront-api' }, profileTemplate({ ...DEFAULT_PROFILE, dimensions: 'horizontal' }), endpoint));
  assert.equal(doc.kind, 'DatadogPodAutoscaler');
  assert.deepEqual(doc.metadata.name, 'storefront-api');
  assert.deepEqual(doc.metadata.namespace, 'checkout');
  assert.deepEqual(JSON.parse(doc.metadata.annotations['autoscaling.datadoghq.com/custom-recommender']), { endpoint });
  assert.deepEqual(doc.spec.targetRef, { apiVersion: 'apps/v1', kind: 'Deployment', name: 'storefront-api' });
  assert.equal(doc.spec.owner, 'Local');
  assert.deepEqual(doc.spec.objectives[0].podResource.value.utilization, 75);
  // Without a recommender, no annotation.
  assert.equal(yaml.load(podAutoscalerManifest({ namespace: 'checkout', kind: 'Deployment', name: 'billing' }, profileTemplate(DEFAULT_PROFILE))).metadata.annotations, undefined);
});

test('commands apply profiles and recommendations, and uninstall autoscalers', () => {
  assert.equal(
    labelCommand({ namespace: 'checkout', kind: 'Deployment', name: 'storefront-api' }, 'checkout-balanced'),
    'kubectl -n checkout label deployment storefront-api autoscaling.datadoghq.com/profile=checkout-balanced --overwrite',
  );
  assert.equal(
    setResourcesCommand({ namespace: 'checkout', kind: 'StatefulSet', name: 'orders-db' }, [
      { name: 'app', recommendedCpu: 0.231, recommendedMemory: 589 * 1024 * 1024 },
      { name: 'sidecar', recommendedCpu: null, recommendedMemory: null },
    ]),
    'kubectl -n checkout set resources statefulset orders-db -c app --requests=cpu=231m,memory=589Mi',
  );
  assert.deepEqual(
    uninstallCommands([
      { namespace: 'checkout', kind: 'DatadogPodAutoscaler', name: 'billing' },
      { namespace: 'catalog', kind: 'HorizontalPodAutoscaler', name: 'legacy-worker' },
    ]),
    ['kubectl -n checkout delete datadogpodautoscaler billing', 'kubectl -n catalog delete horizontalpodautoscaler legacy-worker'],
  );
});

const { filterOverview, summarize, workloadStatus } = await import('../web/src/pages/kubernetes/autoscaling/model.ts');

const workload = (cluster, namespace, name, over = {}) => ({
  uid: `${cluster}/${namespace}/${name}`,
  cluster,
  namespace,
  kind: 'Deployment',
  name,
  replicas: 2,
  containers: [],
  cpuIdle: 0,
  memoryUnused: 0,
  requestsCost: 10,
  usageCost: 4,
  idleCost: 6,
  projectedSavings: 3,
  notReady: [],
  agentSetup: true,
  autoscaler: null,
  ...over,
});
const cluster = (name, over = {}) => ({
  cluster: name,
  autoscaling: 'none',
  nodes: 2,
  cpuCapacity: 8,
  memoryCapacity: 0,
  cpuRequests: 2,
  memoryRequests: 0,
  clusterIdleCost: 50,
  workloadIdleCost: 0,
  usageCost: 0,
  projectedWorkloadSavings: 0,
  projectedClusterSavings: 20,
  ...over,
});
const overview = {
  clusters: [cluster('shop-prod-eu'), cluster('shop-staging', { autoscaling: 'karpenter' })],
  workloads: [
    workload('shop-prod-eu', 'checkout', 'storefront-api'),
    workload('shop-prod-eu', 'catalog', 'catalog-service', { autoscaler: 'DatadogPodAutoscaler/catalog-service' }),
    workload('shop-staging', 'checkout', 'storefront-api'),
  ],
  autoscalers: [{ cluster: 'shop-prod-eu', namespace: 'catalog', name: 'catalog-service' }],
};

test('the summary adds workloads and clusters; a namespace has no cluster idle cost of its own', () => {
  const all = filterOverview(overview, '', '');
  assert.deepEqual(summarize(all.clusters, all.workloads, false), {
    usageCost: 12,
    workloadIdleCost: 18,
    clusterIdleCost: 100,
    workloadSavings: 9,
    clusterSavings: 40,
    workloads: 3,
    unscaledWorkloads: 2,
    clusters: 2,
    unscaledClusters: 1,
  });
  const prod = filterOverview(overview, 'shop-prod-eu', 'checkout');
  assert.deepEqual([prod.clusters.length, prod.workloads.length, prod.autoscalers.length], [1, 1, 0]);
  const s = summarize(prod.clusters, prod.workloads, true);
  assert.deepEqual([s.clusterIdleCost, s.clusterSavings, s.workloads], [0, 0, 1]);
});

test('workloads are ready, not ready or already scaled', () => {
  assert.equal(workloadStatus(workload('c', 'n', 'w')), 'ready');
  assert.equal(workloadStatus(workload('c', 'n', 'w', { notReady: ['No usage metrics'] })), 'not_ready');
  assert.equal(workloadStatus(workload('c', 'n', 'w', { autoscaler: 'HorizontalPodAutoscaler/w', notReady: ['Scaled by HorizontalPodAutoscaler w'] })), 'scaled');
});

test('a saved template opens back in the profile form', async () => {
  const { formFromTemplate } = await import('../web/src/pages/kubernetes/autoscaling/manifests.ts');
  for (const f of [
    DEFAULT_PROFILE,
    { ...DEFAULT_PROFILE, dimensions: 'vertical' },
    { ...DEFAULT_PROFILE, dimensions: 'horizontal', container: 'app', cpuTarget: 60, minReplicas: 3, maxReplicas: 12, scaleDownPercent: 10 },
  ]) {
    const back = formFromTemplate(profileTemplate(f));
    assert.deepEqual(profileTemplate(back), profileTemplate(f));
  }
  // Anything unreadable falls back to the defaults.
  assert.deepEqual(formFromTemplate({}), { ...DEFAULT_PROFILE, dimensions: 'vertical' });
});

test('facets count values and selections filter across facets', async () => {
  const { facetValues, applyFacets } = await import('../web/src/pages/kubernetes/autoscaling/model.ts');
  const items = [
    { cluster: 'shop-prod-eu', issue: 'crash_loop_back_off' },
    { cluster: 'shop-prod-eu', issue: 'oom_killed' },
    { cluster: 'shop-staging', issue: 'crash_loop_back_off' },
  ];
  const keys = { cluster: (i) => i.cluster, issue: (i) => i.issue };
  assert.deepEqual(facetValues(items, keys.cluster), [
    { value: 'shop-prod-eu', count: 2 },
    { value: 'shop-staging', count: 1 },
  ]);
  assert.equal(applyFacets(items, keys, {}).length, 3);
  assert.deepEqual(applyFacets(items, keys, { cluster: ['shop-prod-eu'], issue: ['crash_loop_back_off'] }), [items[0]]);
  assert.equal(applyFacets(items, keys, { cluster: ['shop-prod-eu', 'shop-staging'], issue: [] }).length, 3);
});
