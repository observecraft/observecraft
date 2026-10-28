// In-browser service checks for mock mode: the Datadog Agent's checks on the e-commerce demo hosts,
// with deterministic statuses (a few failing) and messages, shaped like routes/service_checks.rs.
import type { ServiceCheckDetail, ServiceCheckEntry } from '../serviceChecks/client';
import type { TimeRange } from '../types';
import { checkDisplayName } from '../../lib/serviceChecks';

const HOSTS = Array.from({ length: 6 }, (_, i) => `i-0a1b2c3d4e5f6000${i + 1}`);

/** Check → its tag sets per host and the hosts (by index) that misbehave, with how. */
const CHECKS: { check: string; tags: string[][]; bad?: { host: number; status: number; message: string }[] }[] = [
  { check: 'datadog.agent.up', tags: [[]] },
  { check: 'ntp.in_sync', tags: [[]], bad: [{ host: 4, status: 1, message: 'Offset 2.4 s from the NTP server' }] },
  { check: 'disk.read_write.status', tags: [['device:/dev/nvme0n1p1']] },
  { check: 'http.can_connect', tags: [['url:https://storefront.example.com/health', 'instance:storefront-api']], bad: [{ host: 2, status: 2, message: 'Connection timed out after 10 s' }] },
  { check: 'postgres.can_connect', tags: [['db:orders', 'service:order-service']] },
  { check: 'redisdb.can_connect', tags: [['redis_role:master', 'service:catalog-service']] },
  { check: 'kubernetes.kubelet.check', tags: [['kube_cluster_name:shop-prod-eks']] },
  { check: 'kubernetes.kubelet.check.ping', tags: [[]], bad: [{ host: 5, status: 3, message: '' }] },
  { check: 'process.up', tags: [['process:billing-worker', 'service:billing']], bad: [{ host: 1, status: 1, message: 'Found 1 billing-worker process, expected 2' }] },
];

export async function listServiceChecks(): Promise<ServiceCheckEntry[]> {
  return CHECKS.map((c) => ({ check: c.check, name: checkDisplayName(c.check), groups: HOSTS.length * c.tags.length })).sort((a, b) => a.check.localeCompare(b.check));
}

const BUCKETS = 48;

export async function serviceCheckDetail(check: string, range: TimeRange): Promise<ServiceCheckDetail> {
  const c = CHECKS.find((x) => x.check === check);
  if (!c) throw new Error(`No service check named ${check} reported in this time range`);
  const span = Math.max(60_000, range.to - range.from);
  const bucketMs = Math.ceil(Math.max(10_000, Math.ceil(span / BUCKETS)) / 1000) * 1000;
  const first = Math.floor(range.from / bucketMs) * bucketMs;
  const buckets: number[] = [];
  for (let t = first; t <= range.to; t += bucketMs) buckets.push(t);
  const groups = HOSTS.flatMap((host, hi) =>
    c.tags.map((tags) => {
      const bad = c.bad?.find((b) => b.host === hi);
      // A failing host has failed for the last fifth of the range, with a blip earlier on.
      const strip = buckets.map((_, i) => (bad && (i >= buckets.length * 0.8 || i === Math.floor(buckets.length / 3)) ? bad.status : 0));
      return { host, tags, status: bad ? bad.status : 0, strip, message: bad?.message || null };
    }),
  );
  const severity = (s: number) => [0, 2, 3, 1][s] ?? 0;
  groups.sort((a, b) => severity(b.status) - severity(a.status) || a.host.localeCompare(b.host));
  const count = (s: number) => groups.filter((g) => g.status === s).length;
  const latest = c.bad?.find((b) => b.message);
  return {
    check,
    name: checkDisplayName(check),
    counts: { ok: count(0), warning: count(1), critical: count(2), unknown: count(3) },
    buckets,
    bucketMs,
    groups,
    latestMessage: latest ? { host: HOSTS[latest.host], status: latest.status, message: latest.message, at: range.to - 4 * 60_000 } : null,
  };
}
