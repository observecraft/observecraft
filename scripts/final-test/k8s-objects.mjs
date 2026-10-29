#!/usr/bin/env node
// Kubernetes objects for the before/after test (README.md here): a dev-sized inventory (~9.6k
// objects, one cluster) for load-org-1, sent through an intake as OTLP/HTTP JSON logs marked
// `oc.signal = k8s.objects`, the resource attributes the intake gives Datadog orchestrator manifests
// (oc-intake/src/datadog_orch.rs), so they take the same path to otel.otel_k8s_objects.
//
//   node scripts/final-test/k8s-objects.mjs --intake http://localhost:18210 --key ocik_load_org_1_local_only
//   node scripts/final-test/k8s-objects.mjs --print-pods   (the pod names now, to compare with the metrics)
//
// The objects are built from server/crates/oc-api/tests/fixtures/k8s_objects.json (one template per
// kind) with fictional names. The pods are the load profile's pods of the first org (oc-tracegen
// `load`, seed 1, 80,000 series = 3,200 pods; same names, namespaces, deployments, replica sets
// and nodes, churn included), so explorer rows meet their usage metrics. Every 10 s (on the wall
// clock's 10 s boundaries, so two senders agree) one sixth of the objects is sent, each object once
// a minute, plus every pod not sent before (churn) at once. Everything is a function of the time:
// two processes against two intakes send the same objects.
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { gzipSync } from 'node:zlib';

const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};
const intake = arg('--intake', 'http://localhost:4318');
const key = arg('--key', 'ocik_load_org_1_local_only');
const fixture = JSON.parse(readFileSync(new URL('../../server/crates/oc-api/tests/fixtures/k8s_objects.json', import.meta.url)));

// ---- the load profile's pods (server/crates/oc-tracegen/src/load.rs), org index 0
const SEED = 1n;
const ORG = 0n;
const PODS = 3200;
const REPLACED = Math.round(PODS * 0.05);
const CHURN_SECS = 600;
const REPLICAS = 8;
const PODS_PER_NODE = 30;
const SERVICES = [
  'api-gateway',
  'storefront-api',
  'order-service',
  'billing',
  'catalog-service',
  'inventory-service',
  'notification-worker',
  'workflow-engine',
  'payment-service',
  'search-service',
  'cart-service',
  'pricing-service',
  'shipping-service',
  'review-service',
  'recommendation-service',
  'user-service',
];
const NAMESPACES = ['storefront', 'checkout', 'billing', 'catalog', 'inventory', 'notifications', 'search', 'shipping', 'platform', 'analytics'];
const TEAMS = ['storefront', 'checkout', 'payments', 'catalog', 'platform'];
const ZONES = ['us-east-1a', 'us-east-1b', 'us-east-1c'];
const CLUSTER = 'shop-prod-eks';
const M = (1n << 64n) - 1n;
const C = 0x9e3779b97f4a7c15n;

function mix(parts) {
  let h = C;
  for (const p of parts) {
    const part = BigInt(p);
    h ^= (part + C + ((h << 6n) & M) + (h >> 2n)) & M;
    h = ((h ^ (h >> 30n)) * 0xbf58476d1ce4e5b9n) & M;
    h = ((h ^ (h >> 27n)) * 0x94d049bb133111ebn) & M;
    h ^= h >> 31n;
  }
  return h;
}
const hex = (h) => h.toString(16).padStart(16, '0');
const generation = (slot, epoch) => Math.floor((epoch * REPLACED + PODS - 1 - slot) / PODS);

function deployment(di) {
  const service = SERVICES[di % SERVICES.length];
  const name = di < SERVICES.length ? service : `${service}-${String(Math.floor(di / SERVICES.length)).padStart(2, '0')}`;
  return {
    di,
    service,
    name,
    namespace: NAMESPACES[di % NAMESPACES.length],
    replicaSet: `${name}-${hex(mix([SEED, ORG, di])).slice(0, 10)}`,
    version: `${1 + (di % 4)}.${di % 13}.${di % 7}`,
    team: TEAMS[di % TEAMS.length],
    qos: di % 3 === 0 ? 'Guaranteed' : 'Burstable',
  };
}
const nodeName = (index) => `i-0a1b2c3d4e5f${index.toString(16).padStart(5, '0')}`;

function pod(slot, epoch) {
  const gen = generation(slot, epoch);
  const d = deployment(Math.floor(slot / REPLICAS));
  const hash = mix([SEED, ORG, slot, gen]);
  // When this generation started: the epoch whose churn replaced the slot last.
  let since = epoch;
  while (since > 0 && generation(slot, since - 1) === gen) since -= 1;
  return { slot, gen, hash, d, name: `${d.replicaSet}-${hex(hash).slice(0, 5)}`, node: nodeName(Math.floor(slot / PODS_PER_NODE)), zone: ZONES[slot % ZONES.length], since: since * CHURN_SECS };
}

// ---- templates
const template = (kind, name) => {
  const t = fixture.find((o) => o.kind === kind && (!name || o.metadata.name === name)) ?? fixture.find((o) => o.kind === kind);
  if (!t) throw new Error(`no ${kind} in the fixture`);
  return t;
};
const clone = (o) => structuredClone(o);
const uid = (...parts) => {
  const h = createHash('sha1').update(parts.join('/')).digest('hex');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20, 32)}`;
};
const iso = (s) => new Date(s * 1000).toISOString().replace(/\.\d{3}Z$/, 'Z');
const BASE = Date.parse('2026-09-01T00:00:00Z') / 1000;
const appLabels = (d) => ({
  app: d.name,
  'app.kubernetes.io/name': d.name,
  'tags.datadoghq.com/service': d.service,
  'tags.datadoghq.com/env': 'prod',
  'tags.datadoghq.com/version': d.version,
  team: d.team,
});

function meta(o, kind, name, namespace, created, labels) {
  o.kind = kind;
  o.metadata.name = name;
  if (namespace) o.metadata.namespace = namespace;
  else delete o.metadata.namespace;
  o.metadata.uid = uid(kind, namespace ?? '', name, created);
  o.metadata.creationTimestamp = iso(created);
  if (labels) o.metadata.labels = labels;
  delete o.metadata.ownerReferences;
  return o;
}
const owner = (o, kind, apiVersion, name, ownerUid) => {
  o.metadata.ownerReferences = [{ apiVersion, kind, name, uid: ownerUid, controller: true, blockOwnerDeletion: true }];
  return o;
};

function podObject(p, now) {
  const { d } = p;
  const o = meta(clone(template('Pod', 'checkout-api-dc47c7439b-bdf56')), 'Pod', p.name, d.namespace, p.since, { ...appLabels(d), 'pod-template-hash': d.replicaSet.slice(-10) });
  owner(o, 'ReplicaSet', 'apps/v1', d.replicaSet, uid('ReplicaSet', d.namespace, d.replicaSet, BASE + d.di * 60));
  o.spec.nodeName = p.node;
  const c = o.spec.containers[0];
  c.name = d.service;
  c.image = `registry.example.com/shop/${d.service}:${d.version}`;
  c.resources =
    d.qos === 'Guaranteed'
      ? { requests: { cpu: '500m', memory: '512Mi' }, limits: { cpu: '500m', memory: '512Mi' } }
      : { requests: { cpu: '250m', memory: '256Mi' }, limits: { cpu: '1', memory: '1Gi' } };
  o.spec.volumes[0].configMap.name = `${d.name}-config`;
  const ip = Number(p.hash % 65000n);
  o.status.podIP = `10.1.${Math.floor(ip / 250)}.${(ip % 250) + 2}`;
  o.status.hostIP = `10.0.${Math.floor(p.slot / PODS_PER_NODE / 250)}.${(Math.floor(p.slot / PODS_PER_NODE) % 250) + 10}`;
  o.status.qosClass = d.qos;
  o.status.startTime = iso(p.since);
  const cs = o.status.containerStatuses[0];
  cs.name = d.service;
  cs.image = c.image;
  cs.restartCount = Number(p.hash % 7n === 0n ? p.hash % 4n : 0n);
  cs.state = { running: { startedAt: iso(p.since + 5) } };
  cs.containerID = `containerd://${hex(p.hash)}`;
  o.metadata.resourceVersion = String(1e8 + Math.floor(now / 60));
  return o;
}

/** Every object of the cluster at `now` (seconds), in a stable order; pods are the load profile's. */
function objects(now) {
  const epoch = Math.floor(now / CHURN_SECS);
  const out = [];
  const rv = String(1e8 + Math.floor(now / 60));
  const push = (o) => {
    o.metadata.resourceVersion = rv;
    out.push(o);
  };
  for (const ns of [...NAMESPACES, 'kube-system']) push(meta(clone(template('Namespace', 'checkout')), 'Namespace', ns, null, BASE, { 'kubernetes.io/metadata.name': ns }));
  const nodes = Math.ceil(PODS / PODS_PER_NODE);
  for (let n = 0; n < nodes; n++) {
    const o = meta(clone(template('Node', 'ip-10-0-0-10.ec2.internal')), 'Node', nodeName(n), null, BASE + n * 300, {
      'kubernetes.io/os': 'linux',
      'kubernetes.io/hostname': nodeName(n),
      'node.kubernetes.io/instance-type': 'm6i.4xlarge',
      'topology.kubernetes.io/zone': ZONES[n % ZONES.length],
    });
    o.spec.providerID = `aws:///${ZONES[n % ZONES.length]}/${nodeName(n)}`;
    o.status.addresses = [
      { type: 'InternalIP', address: `10.0.${Math.floor(n / 250)}.${(n % 250) + 10}` },
      { type: 'Hostname', address: nodeName(n) },
    ];
    push(o);
  }
  const deployments = PODS / REPLICAS;
  for (let di = 0; di < deployments; di++) {
    const d = deployment(di);
    const created = BASE + di * 60;
    const dep = meta(clone(template('Deployment', 'checkout-api')), 'Deployment', d.name, d.namespace, created, appLabels(d));
    dep.spec.replicas = REPLICAS;
    dep.spec.selector.matchLabels = { app: d.name };
    dep.spec.template.metadata.labels = appLabels(d);
    dep.spec.template.spec.containers[0].name = d.service;
    dep.spec.template.spec.containers[0].image = `registry.example.com/shop/${d.service}:${d.version}`;
    dep.status = { ...dep.status, replicas: REPLICAS, readyReplicas: REPLICAS, updatedReplicas: REPLICAS, availableReplicas: REPLICAS };
    push(dep);
    // The current replica set and two older revisions scaled to zero.
    for (const [rev, rsName] of [
      [0, d.replicaSet],
      [1, `${d.name}-${hex(mix([SEED, ORG, di, 1])).slice(0, 10)}`],
      [2, `${d.name}-${hex(mix([SEED, ORG, di, 2])).slice(0, 10)}`],
    ]) {
      const rs = meta(clone(template('ReplicaSet', 'checkout-api-dc47c7439b')), 'ReplicaSet', rsName, d.namespace, rev === 0 ? created : created - rev * 86400, {
        ...appLabels(d),
        'pod-template-hash': rsName.slice(-10),
      });
      rs.metadata.uid = uid('ReplicaSet', d.namespace, rsName, rev === 0 ? created : created - rev * 86400);
      if (rev === 0) rs.metadata.uid = uid('ReplicaSet', d.namespace, d.replicaSet, created);
      owner(rs, 'Deployment', 'apps/v1', d.name, dep.metadata.uid);
      const n = rev === 0 ? REPLICAS : 0;
      rs.spec.replicas = n;
      rs.spec.selector.matchLabels = { app: d.name, 'pod-template-hash': rsName.slice(-10) };
      rs.status = { replicas: n, readyReplicas: n, availableReplicas: n };
      push(rs);
    }
    const svc = meta(clone(template('Service', 'checkout-api')), 'Service', d.name, d.namespace, created, { app: d.name });
    svc.spec.selector = { app: d.name };
    svc.spec.clusterIP = `172.20.${Math.floor(di / 250)}.${(di % 250) + 2}`;
    push(svc);
    push(meta(clone(template('ServiceAccount', 'storefront-api')), 'ServiceAccount', d.name, d.namespace, created, { app: d.name }));
    const hpa = meta(clone(template('HorizontalPodAutoscaler', 'checkout-api')), 'HorizontalPodAutoscaler', d.name, d.namespace, created, {});
    hpa.spec.scaleTargetRef.name = d.name;
    hpa.spec.minReplicas = REPLICAS;
    hpa.spec.maxReplicas = REPLICAS * 2;
    hpa.status.currentReplicas = REPLICAS;
    hpa.status.desiredReplicas = REPLICAS;
    push(hpa);
    push(meta(clone(template('PodDisruptionBudget', 'storefront-api')), 'PodDisruptionBudget', d.name, d.namespace, created, { app: d.name }));
    push(meta(clone(template('RoleBinding', 'storefront-api-reader')), 'RoleBinding', `${d.name}-reader`, d.namespace, created, { app: d.name }));
    for (let v = 0; v < 2; v++) {
      const claim = `data-${d.name}-${v}`;
      const pvc = meta(clone(template('PersistentVolumeClaim', 'data-storefront-db-0')), 'PersistentVolumeClaim', claim, d.namespace, created, { app: d.name });
      const pvName = `pvc-${uid('pv', claim).slice(0, 23)}`;
      if (pvc.spec) pvc.spec.volumeName = pvName;
      push(pvc);
      const pv = meta(clone(template('PersistentVolume')), 'PersistentVolume', pvName, null, created, {});
      if (pv.spec?.claimRef) Object.assign(pv.spec.claimRef, { name: claim, namespace: d.namespace });
      push(pv);
    }
  }
  for (let slot = 0; slot < PODS; slot++) out.push(podObject(pod(slot, epoch), now));
  for (const [ds, image] of [
    ['kube-proxy', 'registry.example.com/eks/kube-proxy:v1.30.4'],
    ['node-agent', 'registry.example.com/agents/node-agent:7.60.0'],
  ]) {
    const o = meta(clone(template('DaemonSet')), 'DaemonSet', ds, 'kube-system', BASE, { app: ds });
    push(o);
    for (let n = 0; n < nodes; n++) {
      const p = meta(clone(template('Pod', 'kube-proxy-a112e')), 'Pod', `${ds}-${hex(mix([SEED, 7, n, ds.length])).slice(0, 5)}`, 'kube-system', BASE + n * 300, { app: ds });
      owner(p, 'DaemonSet', 'apps/v1', ds, o.metadata.uid);
      p.spec.nodeName = nodeName(n);
      if (p.spec.containers?.[0]) p.spec.containers[0].image = image;
      push(p);
    }
  }
  for (let cj = 0; cj < 100; cj++) {
    const ns = NAMESPACES[cj % NAMESPACES.length];
    const name = `${ns}-report-${Math.floor(cj / NAMESPACES.length)}`;
    const cron = meta(clone(template('CronJob', 'storefront-report-0')), 'CronJob', name, ns, BASE, { app: name });
    push(cron);
    // The last three runs (hourly), with their pods.
    const hour = Math.floor(now / 3600);
    for (let k = 1; k <= 3; k++) {
      const jobName = `${name}-${(hour - k) * 60}`;
      const job = owner(meta(clone(template('Job', 'storefront-report-0-29000000')), 'Job', jobName, ns, (hour - k) * 3600, { app: name }), 'CronJob', 'batch/v1', name, cron.metadata.uid);
      push(job);
      const jp = owner(
        meta(clone(template('Pod', 'storefront-report-0-29000001-d4353')), 'Pod', `${jobName}-${hex(mix([SEED, cj, hour - k])).slice(0, 5)}`, ns, (hour - k) * 3600, {
          app: name,
          'job-name': jobName,
        }),
        'Job',
        'batch/v1',
        jobName,
        job.metadata.uid,
      );
      jp.spec.nodeName = nodeName((cj * 3 + k) % nodes);
      push(jp);
    }
  }
  for (let i = 0; i < 40; i++) {
    const d = deployment(i);
    push(meta(clone(template('Ingress', 'storefront-web')), 'Ingress', d.name, d.namespace, BASE, { app: d.name }));
  }
  for (const ns of NAMESPACES) {
    push(meta(clone(template('NetworkPolicy')), 'NetworkPolicy', `${ns}-default-deny`, ns, BASE, {}));
    push(meta(clone(template('ResourceQuota')), 'ResourceQuota', `${ns}-quota`, ns, BASE, {}));
    push(meta(clone(template('LimitRange')), 'LimitRange', `${ns}-limits`, ns, BASE, {}));
    push(meta(clone(template('Role')), 'Role', `${ns}-reader`, ns, BASE, {}));
  }
  for (let i = 0; i < 20; i++) {
    push(meta(clone(template('ClusterRole')), 'ClusterRole', `system:controller:role-${i}`, null, BASE, {}));
    push(meta(clone(template('ClusterRoleBinding')), 'ClusterRoleBinding', `system:controller:role-${i}`, null, BASE, {}));
  }
  for (let i = 0; i < 10; i++) push(meta(clone(template('CustomResourceDefinition')), 'CustomResourceDefinition', `widgets${i}.shop.example.com`, null, BASE, {}));
  for (const sc of ['gp2', 'gp3']) push(meta(clone(template('StorageClass')), 'StorageClass', sc, null, BASE, {}));
  return out;
}

if (args.includes('--print-pods')) {
  const now = Math.floor(Date.now() / 1000);
  const epoch = Math.floor(now / CHURN_SECS);
  for (let slot = 0; slot < PODS; slot++) console.log(pod(slot, epoch).name);
  process.exit(0);
}
if (args.includes('--count')) {
  const all = objects(Math.floor(Date.now() / 1000));
  const kinds = {};
  for (const o of all) kinds[o.kind] = (kinds[o.kind] ?? 0) + 1;
  console.log(all.length, JSON.stringify(kinds), Math.round(all.reduce((s, o) => s + JSON.stringify(o).length, 0) / 1e6), 'MB');
  process.exit(0);
}

async function send(batch, now) {
  const logRecords = batch.map((o) => ({ timeUnixNano: `${now}000000000`, observedTimeUnixNano: `${now}000000000`, body: { stringValue: JSON.stringify(o) } }));
  const body = {
    resourceLogs: [
      {
        resource: {
          attributes: [
            { key: 'k8s.cluster.name', value: { stringValue: CLUSTER } },
            { key: 'oc.signal', value: { stringValue: 'k8s.objects' } },
            { key: 'observecraft.collector', value: { stringValue: 'datadog-cluster-agent' } },
          ],
        },
        scopeLogs: [{ scope: { name: 'datadog.orchestrator' }, logRecords }],
      },
    ],
  };
  const res = await fetch(`${intake}/v1/logs`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'content-encoding': 'gzip', authorization: `Bearer ${key}` },
    body: gzipSync(JSON.stringify(body)),
  });
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
}

const sent = new Set();
let first = true;
for (;;) {
  const wait = 10_000 - (Date.now() % 10_000);
  await new Promise((r) => setTimeout(r, wait));
  const now = Math.floor(Date.now() / 1000);
  const slice = Math.floor(now / 10) % 6;
  const all = objects(now);
  // The first tick sends everything (an agent's start), then a sixth per tick plus unseen objects.
  const due = all.filter((o, i) => first || i % 6 === slice || !sent.has(o.metadata.uid));
  first = false;
  for (const o of due) sent.add(o.metadata.uid);
  const started = Date.now();
  let failed = 0;
  for (let i = 0; i < due.length; i += 500) {
    try {
      await send(due.slice(i, i + 500), now);
    } catch (e) {
      failed += 1;
      console.error(new Date().toISOString(), 'send failed', e.message);
    }
  }
  console.log(new Date().toISOString(), `sent ${due.length} of ${all.length} objects in ${Date.now() - started} ms${failed ? `, ${failed} batches failed` : ''}`);
}
