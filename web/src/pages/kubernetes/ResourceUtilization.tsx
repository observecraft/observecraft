import { useEffect, useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { ArrowDownWideNarrow, ArrowUpNarrowWide, Search, X } from 'lucide-react';
import { queryTimeseries } from '../../api/dashboards/client';
import { searchKubernetes, type KubeGroup } from '../../api/kubernetes/client';
import type { MetricSeries } from '../../api/types';
import { Chart } from '../../components/charts/Chart';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { usePopover } from '../../components/ui/Dropdown';
import { parseQuery } from '../../lib/searchQuery';
import { TAG_LABELS, UtilCell, formatCores, formatCoresAxis, formatMem } from './columns';

// Datadog's Kubernetes Overview → Resource Utilization: pods grouped (by default) by cluster,
// namespace and deployment, with CPU and Memory tabs showing idle capacity and usage against
// requests and limits, to right-size workloads.

type Res = 'cpu' | 'memory';
const DEFAULT_GROUP = ['kube_cluster_name', 'kube_namespace', 'kube_deployment'];
const HOUR = 3600e3;

const METRICS: Record<Res, { usage: string; requests: string; limits: string; container: string }> = {
  cpu: { usage: 'k8s.pod.cpu.utilization', requests: 'k8s.container.cpu_request', limits: 'k8s.container.cpu_limit', container: 'container.cpu.utilization' },
  memory: { usage: 'k8s.pod.memory.working_set', requests: 'k8s.container.memory_request', limits: 'k8s.container.memory_limit', container: 'container.memory.working_set' },
};

/** Single-value, non-negated tag terms of the filter, as a metric scope (`a:b,c:d`). */
function metricScope(query: string, extra: string[] = []): string {
  const tags = parseQuery(query).flatMap((t) => (t.kind === 'tag' && !t.negate && t.values.length === 1 ? [`${t.key}:${t.values[0]}`] : []));
  const all = [...tags, ...extra];
  return all.length ? all.join(',') : '*';
}

/** A group's members as tags; groups without a value (`no kube_deployment`) are scoped by the others. */
function groupTags(keys: string[], g: KubeGroup): string[] {
  return keys.flatMap((k, i) => (g.values[i] && !g.values[i].startsWith('no ') ? [`${k}:${g.values[i]}`] : []));
}

/** Series name of a `by {keys}` metric query for this group. */
function seriesName(keys: string[], g: KubeGroup): string {
  // The query service names series `k1:v1, k2:v2` with the keys sorted.
  return keys
    .map((k, i) => `${k}:${g.values[i]?.startsWith('no ') ? 'N/A' : g.values[i]}`)
    .sort()
    .join(', ');
}

function pctClass(v: number): string {
  // Over the request: under-provisioned (red); far below: over-provisioned (blue); near: right-sized.
  return v > 100 ? 'kx-rs-under' : v >= 60 ? 'kx-rs-good' : v >= 25 ? 'kx-rs-ok' : 'kx-rs-over';
}

function Pct({ v }: { v: number | null }) {
  if (v == null) return <span className="muted">—</span>;
  return <span className={`kx-rs-pct ${pctClass(v)}`}>{v < 1 ? '<1' : Math.round(v)}%</span>;
}

function GroupBy({ value, keys, onChange }: { value: string[]; keys: string[]; onChange: (v: string[]) => void }) {
  const { open, setOpen, ref } = usePopover();
  const options = [...new Set([...value, ...DEFAULT_GROUP, ...keys.filter((k) => !k.includes('/') || k.startsWith('app.kubernetes.io/'))])];
  return (
    <div className="dd kx-rs-groupby" ref={ref}>
      <button className="kx-rs-groupby-trigger" onClick={() => setOpen((o) => !o)}>
        {value.length ? (
          value.map((k) => (
            <span key={k} className="kx-chip">
              {k}
            </span>
          ))
        ) : (
          <span className="muted">pod_name</span>
        )}
      </button>
      {open && (
        <div className="dd-menu dd-right options-menu-list kx-rs-groupby-menu">
          {options.map((k) => (
            <label key={k}>
              <input type="checkbox" className="checkbox" checked={value.includes(k)} onChange={() => onChange(value.includes(k) ? value.filter((x) => x !== k) : [...value, k])} />
              {k}
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

function Sparkline({ usage, requests, limits, res }: { usage?: MetricSeries; requests: number | null; limits: number | null; res: Res }) {
  const option = useMemo<EChartsOption>(() => {
    const pts = usage?.points ?? [];
    const flat = (v: number | null) => (v == null || pts.length === 0 ? [] : [[pts[0][0], v] as [number, number], [pts[pts.length - 1][0], v] as [number, number]]);
    const fmt = (v: unknown) => (res === 'cpu' ? formatCores(Number(v)) : formatMem(Number(v)));
    return {
      grid: { left: 2, right: 2, top: 4, bottom: 4 },
      tooltip: { trigger: 'axis', valueFormatter: fmt },
      xAxis: { type: 'time', show: false },
      yAxis: { type: 'value', show: false, min: 0 },
      series: [
        { name: 'usage', type: 'line', showSymbol: false, areaStyle: { opacity: 0.25 }, lineStyle: { width: 1.2 }, data: pts },
        { name: 'requests', type: 'line', showSymbol: false, lineStyle: { width: 1, type: 'dashed' }, data: flat(requests) },
        { name: 'limits', type: 'line', showSymbol: false, lineStyle: { width: 1, color: '#d9434b' }, itemStyle: { color: '#d9434b' }, data: flat(limits) },
      ],
    };
  }, [usage, requests, limits, res]);
  if (!usage) return <span className="muted">—</span>;
  return <Chart option={option} height={34} />;
}

export function ResourceUtilization({ query, setQuery, res, setRes }: { query: string; setQuery: (q: string) => void; res: Res; setRes: (r: Res) => void }) {
  const [draft, setDraft] = useState(query);
  const [groupBy, setGroupBy] = useState<string[]>(DEFAULT_GROUP);
  const [sort, setSort] = useState<{ key: string; desc: boolean }>({ key: 'idle', desc: true });
  const [open, setOpen] = useState<KubeGroup | null>(null);
  const keys = groupBy.length ? groupBy : ['pod_name'];
  const groupsQ = useQuery({
    queryKey: ['kubernetes', 'utilization', query, keys],
    queryFn: () => searchKubernetes({ kind: 'pods', query, groupBy: keys.join(','), limit: 1 }),
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
  const range = useMemo(() => {
    const to = Date.now();
    return { from: to - HOUR, to };
    // A new one-hour window each time the groups refresh.
  }, [groupsQ.dataUpdatedAt]);
  const usageQ = useQuery({
    queryKey: ['kubernetes', 'utilization', 'usage', res, query, keys, range],
    queryFn: () => queryTimeseries({ queries: [`sum:${METRICS[res].usage}{${metricScope(query)}} by {${keys.join(',')}}`], range, points: 30 }),
  });
  const series = useMemo(() => new Map((usageQ.data ?? []).map((s) => [s.name, s])), [usageQ.data]);

  const rows = useMemo(() => {
    const list = (groupsQ.data?.groups ?? []).map((g) => {
      const u = g[res];
      const ok = res === 'cpu' ? g.cpuComplete : g.memoryComplete;
      const usage = ok ? (u.usage ?? null) : null;
      const req = ok ? (u.requests ?? null) : null;
      const lim = ok ? (u.limits ?? null) : null;
      return {
        g,
        ok,
        usage,
        req,
        lim,
        idle: usage != null && req != null ? req - usage : null,
        pctReq: usage != null && req ? (usage / req) * 100 : null,
        pctLim: usage != null && lim ? (usage / lim) * 100 : null,
      };
    });
    type Row = (typeof list)[number];
    const val = (r: Row): number | string => (sort.key === 'group' ? r.g.key : ((r[sort.key as keyof Row] as number | null) ?? -Infinity));
    // Groups with a container lacking requests or limits have no metrics and stay last.
    return list.sort((a, b) => {
      if (a.ok !== b.ok) return a.ok ? -1 : 1;
      const x = val(a);
      const y = val(b);
      const o = x < y ? -1 : x > y ? 1 : 0;
      return sort.desc ? -o : o;
    });
  }, [groupsQ.data, res, sort]);

  const fmt = res === 'cpu' ? formatCores : formatMem;
  const label = res === 'cpu' ? 'CPU' : 'MEMORY';
  const th = (key: string, text: string, num = true) => (
    <th className={`sortable${num ? ' num' : ''}`} onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : true }))}>
      {text} {sort.key === key && (sort.desc ? <ArrowDownWideNarrow size={13} /> : <ArrowUpNarrowWide size={13} />)}
    </th>
  );

  return (
    <div className="hosts-scroll ox-scroll">
      <div>
        <div className="hosts-search ox-search">
          <span className="hosts-groupby-label ox-search-label">Filter by</span>
          <label className="hosts-search-input">
            <Search size={15} />
            <input
              value={draft}
              placeholder="kube_cluster_name:prod kube_namespace:shop"
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && setQuery(draft.trim())}
              onBlur={() => draft.trim() !== query && setQuery(draft.trim())}
            />
          </label>
          <span className="hosts-groupby-label">Group by</span>
          <GroupBy value={groupBy} keys={groupsQ.data?.tagKeys ?? []} onChange={setGroupBy} />
        </div>
      </div>
      <div className="kx-rs-tabs">
        <button className={`kx-tab${res === 'cpu' ? ' active' : ''}`} onClick={() => setRes('cpu')}>
          CPU
        </button>
        <button className={`kx-tab${res === 'memory' ? ' active' : ''}`} onClick={() => setRes('memory')}>
          Memory
        </button>
      </div>
      {groupsQ.isPending ? (
        <Loading label="Loading pods…" />
      ) : groupsQ.isError ? (
        <ErrorState error={groupsQ.error} onRetry={() => groupsQ.refetch()} />
      ) : rows.length === 0 ? (
        <Empty>No pods match these filters.</Empty>
      ) : (
        <div className="kx-table-wrap">
          <table className="host-table ox-table kx-rs-table">
            <thead>
              <tr>
                {th('group', 'POD GROUP', false)}
                {th('idle', res === 'cpu' ? 'CPU IDLE' : 'MEMORY UNUSED')}
                {th('pctReq', `${label} USAGE/REQUESTS`)}
                {th('pctLim', `${label} USAGE/LIMITS`)}
                <th>{label} GRAPH</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.g.key} className={open?.key === r.g.key ? 'selected' : undefined} onClick={() => setOpen(r.g)}>
                  <td className="kx-rs-group">
                    <div className="kx-rs-tags">
                      {keys.map((k, i) => (
                        <span key={k} className="kx-chip" title={TAG_LABELS[k] ?? k}>
                          {k}:{r.g.values[i]?.startsWith('no ') ? 'N/A' : r.g.values[i]}
                        </span>
                      ))}
                    </div>
                    <div className="kx-rs-sum muted">
                      {r.g.count} pod{r.g.count === 1 ? '' : 's'}
                      {r.ok ? ` · usage ${fmt(r.usage)} · requests ${fmt(r.req)} · limits ${fmt(r.lim)}` : ' · a container has no requests or limits'}
                    </div>
                  </td>
                  <td className="num">{r.idle == null ? '—' : fmt(Math.max(0, r.idle))}</td>
                  <td className="num">
                    <Pct v={r.pctReq} />
                  </td>
                  <td className="num">
                    <Pct v={r.pctLim} />
                  </td>
                  <td className="kx-rs-graph">{r.ok ? <Sparkline usage={series.get(seriesName(keys, r.g))} requests={r.req} limits={r.lim} res={res} /> : <span className="muted">—</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {open && <GroupPanel group={open} keys={keys} query={query} res={res} onClose={() => setOpen(null)} />}
    </div>
  );
}

/** Row side panel: the group's CPU and memory by pod (or container) and its top pods. */
function GroupPanel({ group, keys, query, res, onClose }: { group: KubeGroup; keys: string[]; query: string; res: Res; onClose: () => void }) {
  const [by, setBy] = useState<'pod_name' | 'kube_container_name'>('pod_name');
  const tags = groupTags(keys, group);
  const scope = metricScope(query, tags);
  const range = useMemo(() => {
    const to = Date.now();
    return { from: to - 4 * HOUR, to };
  }, []);
  const pods = useQuery({
    queryKey: ['kubernetes', 'utilization', 'pods', query, tags, res],
    queryFn: () => searchKubernetes({ kind: 'pods', query: [query, ...tags].filter(Boolean).join(' '), sort: res, desc: true, limit: 10 }),
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <aside className="host-panel kx-panel" role="dialog" aria-label="Pod group">
      <div className="kx-panel-scroll">
        <header className="kx-panel-head">
          <div className="kx-panel-kind">Pod group</div>
          <div className="kx-panel-title">
            <div className="kx-rs-tags">
              {tags.map((t) => (
                <span key={t} className="kx-chip">
                  {t}
                </span>
              ))}
            </div>
            <button className="icon-plain kx-panel-close" title="Close (Esc)" onClick={onClose}>
              <X size={18} />
            </button>
          </div>
          <dl className="kx-fields kx-rs-fields">
            {(['cpu', 'memory'] as const).map((k) => (
              <div className="kx-field" key={k}>
                <dt>{k === 'cpu' ? 'CPU USAGE / REQUESTS' : 'MEMORY USAGE / REQUESTS'}</dt>
                <dd>{(k === 'cpu' ? group.cpuComplete : group.memoryComplete) ? <UtilCell u={group[k]} kind={k} /> : <span className="muted">a container has no requests or limits</span>}</dd>
              </div>
            ))}
          </dl>
        </header>
        <div className="kx-panel-body">
          <div className="kx-rs-by">
            Graph by
            <button className={`btn btn-sm${by === 'pod_name' ? ' active' : ''}`} onClick={() => setBy('pod_name')}>
              pod_name
            </button>
            <button className={`btn btn-sm${by === 'kube_container_name' ? ' active' : ''}`} onClick={() => setBy('kube_container_name')}>
              kube_container_name
            </button>
          </div>
          {(['cpu', 'memory'] as const).map((k) => (
            <GroupChart
              key={k + by}
              title={k === 'cpu' ? 'CPU usage' : 'Memory usage'}
              query={`sum:${by === 'pod_name' ? METRICS[k].usage : METRICS[k].container}{${scope}} by {${by}}`}
              res={k}
              range={range}
            />
          ))}
          <h4 className="kx-rs-top">Top pods by {res === 'cpu' ? 'CPU' : 'memory'} usage</h4>
          {pods.isPending ? (
            <Loading />
          ) : pods.isError ? (
            <ErrorState error={pods.error} onRetry={() => pods.refetch()} />
          ) : (
            <table className="kx-mini">
              <thead>
                <tr>
                  <th>POD</th>
                  <th>{res === 'cpu' ? 'CPU' : 'MEMORY'} USAGE / REQUESTS</th>
                </tr>
              </thead>
              <tbody>
                {pods.data.items.map((p) => (
                  <tr key={p.uid}>
                    <td>{p.name}</td>
                    <td>
                      <UtilCell u={p[res]} kind={res} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </aside>
  );
}

function GroupChart({ title, query, res, range }: { title: string; query: string; res: Res; range: { from: number; to: number } }) {
  const q = useQuery({ queryKey: ['kubernetes', 'utilization', 'chart', query, range], queryFn: () => queryTimeseries({ queries: [query], range, points: 80 }) });
  const option = useMemo<EChartsOption>(() => {
    const fmt = (v: unknown) => (res === 'cpu' ? formatCores(Number(v)) : formatMem(Number(v)));
    return {
      grid: { left: 56, right: 10, top: 12, bottom: 46 },
      tooltip: { trigger: 'axis', valueFormatter: fmt },
      legend: { type: 'scroll', bottom: 0, textStyle: { fontSize: 11 } },
      xAxis: { type: 'time', splitNumber: 4, axisLabel: { hideOverlap: true } },
      yAxis: { type: 'value', axisLabel: { formatter: (v: number) => (res === 'cpu' ? formatCoresAxis(v) : formatMem(v)) } },
      series: (q.data ?? []).map((s) => ({ name: s.name, type: 'line', showSymbol: false, lineStyle: { width: 1.4 }, data: s.points })),
    };
  }, [q.data, res]);
  return (
    <section className="kx-chart">
      <h4>{title}</h4>
      <code className="kx-chart-query">{query}</code>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.length === 0 ? (
        <Empty>No data in the past 4 hours.</Empty>
      ) : (
        <Chart option={option} height={200} />
      )}
    </section>
  );
}
