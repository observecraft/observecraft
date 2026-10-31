// Runs inside the isolated network. Real intake requests; no sender retries or direct inserts.
import { randomUUID } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import { distribution } from './model.mjs';

const cfg = JSON.parse(process.argv[2]);
const token = randomUUID();
const requestMs = [],
  scheduleLagMs = [],
  probeMs = [];
const probes = new Map();
const active = new Set();
const statuses = {};
const acknowledgedBatches = {};
let accepted = 0,
  failed = 0,
  sent = 0,
  bytes = 0,
  peakInFlight = 0;
let probeReadsFailed = 0,
  loading = true;
const start = performance.now();
const startedAt = new Date().toISOString();
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const orgClause = cfg.orgs.map((_, i) => `{org${i}:String}`).join(',');
async function readProbes() {
  const url = new URL('http://clickhouse:8123/');
  url.searchParams.set('query', `SELECT JSONExtractString(attributes, 'bench.probe') AS probe FROM oc.logs_read WHERE org_id IN (${orgClause}) AND service = 'bench-probe' FORMAT JSONEachRow`);
  cfg.orgs.forEach((org, i) => url.searchParams.set(`param_org${i}`, org));
  const r = await fetch(url, { signal: AbortSignal.timeout(5000) });
  if (!r.ok) throw new Error(await r.text());
  for (const line of (await r.text()).trim().split('\n').filter(Boolean)) {
    const id = JSON.parse(line).probe;
    const probe = probes.get(id);
    if (probe && !probe.seen) {
      probe.seen = true;
      probeMs.push(performance.now() - probe.started);
    }
  }
}
const observer = (async () => {
  while (loading) {
    try {
      await readProbes();
    } catch {
      probeReadsFailed++;
    }
    await sleep(500);
  }
})();
async function send(batch, ordinal, due) {
  const orgIndex = ordinal % cfg.orgs.length;
  const id = `${token}-${ordinal}`;
  const begun = performance.now();
  scheduleLagMs.push(Math.max(0, begun - due));
  const timestamp = new Date().toISOString();
  const rows = Array.from({ length: batch }, (_, n) => ({
    timestamp,
    service: n === 0 ? 'bench-probe' : ['order-service', 'billing', 'catalog-service', 'storefront-api'][n % 4],
    status: n % 20 === 0 ? 'error' : 'info',
    host: `bench-node-${n % 8}`,
    message: `order accepted ${'x'.repeat(cfg.messageBytes)}`,
    attributes: { 'bench.request': id, ...(n === 0 ? { 'bench.probe': id } : {}) },
    ddtags: 'env:benchmark,team:commerce',
  }));
  const probe = { started: begun, seen: false, accepted: false };
  probes.set(id, probe);
  const body = JSON.stringify(rows);
  bytes += Buffer.byteLength(body);
  try {
    const r = await fetch('http://intake:4318/api/v2/logs', {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${cfg.keys[orgIndex]}` },
      body,
      signal: AbortSignal.timeout(30000),
    });
    await r.text();
    statuses[r.status] = (statuses[r.status] ?? 0) + 1;
    if (r.ok) {
      accepted += batch;
      acknowledgedBatches[id] = batch;
      probe.accepted = true;
    } else failed += batch;
  } catch (e) {
    statuses[e.name] = (statuses[e.name] ?? 0) + 1;
    failed += batch;
  }
  requestMs.push(performance.now() - begun);
}
let ordinal = 0;
while (sent < cfg.total) {
  const due = start + (sent / cfg.rate) * 1000;
  if (due > performance.now()) await sleep(due - performance.now());
  while (active.size >= cfg.concurrency) await Promise.race(active);
  const batch = Math.min(cfg.batch, cfg.total - sent);
  const job = send(batch, ordinal++, due).finally(() => active.delete(job));
  active.add(job);
  peakInFlight = Math.max(peakInFlight, active.size);
  sent += batch;
}
await Promise.all(active);
// Batches are scheduled at the start of each interval; include the final interval in the offered
// window so small tests do not overstate achieved rate merely by front-loading their last batch.
const offerEnd = start + (cfg.total / cfg.rate) * 1000;
if (performance.now() < offerEnd) await sleep(offerEnd - performance.now());
const sendSeconds = (performance.now() - start) / 1000;
const sendFinishedAt = new Date().toISOString();
// Probe observation is separate from intake ACK latency; first visibility includes 500ms polling.
const deadline = performance.now() + 90000;
while ([...probes.values()].some((p) => p.accepted && !p.seen) && performance.now() < deadline) await sleep(500);
loading = false;
await observer;
console.log(
  JSON.stringify({
    startedAt,
    runToken: token,
    acknowledgedBatches,
    sendFinishedAt,
    offered: sent,
    accepted,
    failed,
    payloadBytes: bytes,
    sendSeconds,
    acceptedPerSecond: accepted / sendSeconds,
    statuses,
    peakInFlight,
    intakeAckMs: distribution(requestMs),
    scheduleLagMs: distribution(scheduleLagMs),
    indexedProbeVisibilityMs: distribution(probeMs),
    missingAcceptedProbes: [...probes.values()].filter((p) => p.accepted && !p.seen).length,
    probeReadsFailed,
  }),
);
