// Infrastructure data access. In "live" mode hosts come from the Rust API (host metrics, spans and
// logs in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/hosts/search                 searchHosts
//   POST /api/v1/hosts/:name                  getHost        (range in body)
//   POST /api/v1/hosts/:name/metrics          getHostMetrics
//   POST /api/v1/hosts/:name/pods|containers|processes
import type { components } from '../generated/schema';
import { API_MODE, post, postTo } from '../http';
import * as mock from '../mock/infrastructure';
import type { TimeRange } from '../types';
import type { Container, HostDetail, HostMetrics, HostSearchPage, HostSearchRequest, HostSummary, PageRequest, Paged, Pod, ProcessPage, TriggeredMonitor } from './types';

const live = API_MODE === 'live';

/** Host detail has no range of its own; the panel shows the host as of the last hour. */
const HOUR = 3600e3;
const lastHour = (): TimeRange => {
  const to = Date.now();
  return { from: to - HOUR, to };
};

type WireSummary = components['schemas']['HostSummary'];

const toSummary = (h: WireSummary): HostSummary => ({ ...h, distro: h.distro ?? undefined, env: h.env ?? null, service: h.service ?? null, team: h.team ?? null, errorSpans: h.errorSpans ?? null });

export async function searchHosts(req: HostSearchRequest): Promise<HostSearchPage> {
  if (!live) return mock.searchHosts(req);
  const page = await post('/api/v1/hosts/search', req);
  return { total: page.total, groups: page.groups ?? undefined, hosts: page.hosts.map((h) => ({ ...toSummary(h), group: h.group ?? undefined })) };
}

export async function getHost(name: string): Promise<HostDetail> {
  if (!live) return mock.getHost(name);
  const d = await postTo('/api/v1/hosts/{name}', { name }, { range: lastHour() });
  return {
    ...d,
    ...toSummary(d),
    distro: d.distro ?? undefined,
    account: d.account ?? null,
    region: d.region ?? null,
    kubernetes: d.kubernetes ?? null,
    triggeredMonitors: d.triggeredMonitors.map((m) => ({ ...m, status: m.status as TriggeredMonitor['status'] })),
    integrations: d.integrations.map((i) => ({ ...i, status: i.status as 'ok' | 'broken' })),
  };
}

export async function getHostMetrics(name: string, range: TimeRange): Promise<HostMetrics> {
  if (!live) return mock.getHostMetrics(name, range);
  return postTo('/api/v1/hosts/{name}/metrics', { name }, { range });
}

const page = (req: PageRequest) => ({ query: req.query, offset: req.offset, limit: req.limit });

export async function listPods(req: PageRequest): Promise<Paged<Pod>> {
  if (!live) return mock.listPods(req);
  const r = await postTo('/api/v1/hosts/{name}/pods', { name: req.host }, page(req));
  return { items: r.items as unknown as Pod[], total: r.total };
}

export async function listContainers(req: PageRequest): Promise<Paged<Container>> {
  if (!live) return mock.listContainers(req);
  const r = await postTo('/api/v1/hosts/{name}/containers', { name: req.host }, page(req));
  return { items: r.items as unknown as Container[], total: r.total };
}

export async function listProcesses(req: PageRequest): Promise<ProcessPage> {
  if (!live) return mock.listProcesses(req);
  return postTo('/api/v1/hosts/{name}/processes', { name: req.host }, page(req));
}
