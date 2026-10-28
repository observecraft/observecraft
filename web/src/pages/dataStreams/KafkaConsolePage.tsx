import { Fragment, useMemo, useState, type ReactNode } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { ChevronDown, ChevronRight, CircleCheck, CircleDashed, CircleX, HeartPulse, Plus, Search, Settings2, X } from 'lucide-react';
import type { KafkaBroker, KafkaCluster, KafkaConnector, KafkaConsole, KafkaConsumerGroup, KafkaHealthCheck, KafkaTopic } from '../../api/dataStreams/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select, usePopover } from '../../components/ui/Dropdown';
import { timeAgo } from '../../lib/format';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { fmtBytesRate, fmtCount, fmtLagSeconds, fmtRate } from './format';
import { LineChart } from './LineChart';
import { useKafkaConsole } from './queries';

type Tab = 'clusters' | 'topics' | 'brokers' | 'groups' | 'connectors';
const TABS: { id: Tab; label: string; noun: string }[] = [
  { id: 'clusters', label: 'Clusters', noun: 'clusters' },
  { id: 'topics', label: 'Topics', noun: 'topics' },
  { id: 'brokers', label: 'Brokers', noun: 'brokers' },
  { id: 'groups', label: 'Consumer Groups', noun: 'consumer groups' },
  { id: 'connectors', label: 'Connectors', noun: 'connectors' },
];

type FilterId = 'failing' | 'lag' | 'under_replicated' | 'failed_tasks';
const FILTERS: { id: FilterId; label: string }[] = [
  { id: 'failing', label: 'Failing health checks' },
  { id: 'lag', label: 'High consumer lag' },
  { id: 'under_replicated', label: 'Under-replicated partitions' },
  { id: 'failed_tasks', label: 'Failed connector tasks' },
];

interface Column<T> {
  key: string;
  label: string;
  /** Columns sharing a group get a common header above theirs. */
  group?: string;
  num?: boolean;
  render: (row: T) => ReactNode;
  sort?: (row: T) => number | string;
}

const store = {
  get(key: string): string | null {
    try {
      return window.localStorage.getItem(key);
    } catch {
      return null;
    }
  },
  set(key: string, value: string) {
    try {
      window.localStorage.setItem(key, value);
    } catch {
      // Storage unavailable (private mode): the choice lasts for this page only.
    }
  },
};

const n = (v: number | null | undefined) => (v == null ? '—' : fmtCount(v));

// ================================================================ filtering

interface Scope {
  needle: string;
  cluster: string;
  filters: FilterId[];
}

/** Rows of every tab that pass the search, the cluster and the filters. */
export function scopeConsole(k: KafkaConsole, s: Scope) {
  const needle = s.needle.trim().toLowerCase();
  const has = (f: FilterId) => s.filters.includes(f);
  const inCluster = (c: string) => !s.cluster || c === s.cluster;
  const text = (...parts: (string | null | undefined)[]) => !needle || parts.some((p) => p?.toLowerCase().includes(needle));
  const laggingGroups = new Set(k.consumerGroups.filter((g) => g.lagHigh).map((g) => `${g.cluster}|${g.name}`));
  const failingClusters = new Set(k.clusters.filter((c) => c.checksFailing > 0).map((c) => c.name));
  const lagClusters = new Set(k.consumerGroups.filter((g) => g.lagHigh).map((g) => g.cluster));
  const taskClusters = new Set(k.connectors.filter((c) => c.failed > 0).map((c) => c.cluster));
  const clusterOk = (c: string) => inCluster(c) && (!has('failing') || failingClusters.has(c)) && (!has('lag') || lagClusters.has(c)) && (!has('failed_tasks') || taskClusters.has(c));
  const clusters = k.clusters.filter((c) => clusterOk(c.name) && (!has('under_replicated') || (c.underReplicated ?? 0) > 0) && text(c.name));
  const topics = k.topics.filter(
    (t) =>
      clusterOk(t.cluster) &&
      (!has('under_replicated') || (t.underReplicated ?? 0) > 0) &&
      (!has('lag') || t.consumerGroups.some((g) => laggingGroups.has(`${t.cluster}|${g}`))) &&
      text(t.name, t.cluster),
  );
  const brokers = k.brokers.filter((b) => clusterOk(b.cluster) && (!has('under_replicated') || (b.underReplicated ?? 0) > 0) && text(b.id, b.host, b.cluster));
  const groups = k.consumerGroups.filter((g) => clusterOk(g.cluster) && (!has('lag') || g.lagHigh) && text(g.name, g.cluster, ...g.topics.map((t) => t.topic)));
  const connectors = k.connectors.filter((c) => clusterOk(c.cluster) && (!has('failed_tasks') || c.failed > 0) && text(c.name, c.cluster, c.class));
  return { clusters, topics, brokers, groups, connectors };
}

// ================================================================ table

function DataTable<T>({ columns, rows, rowKey, hidden, expand }: { columns: Column<T>[]; rows: T[]; rowKey: (r: T) => string; hidden: Set<string>; expand?: (r: T) => ReactNode }) {
  const visible = columns.filter((c) => !hidden.has(c.key));
  const [sort, setSort] = useState<{ key: string; desc: boolean } | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const sorted = useMemo(() => {
    const col = sort && columns.find((c) => c.key === sort.key);
    if (!col?.sort) return rows;
    const f = col.sort;
    return [...rows].sort((a, b) => {
      const x = f(a);
      const y = f(b);
      const cmp = typeof x === 'number' && typeof y === 'number' ? x - y : String(x).localeCompare(String(y));
      return sort!.desc ? -cmp : cmp;
    });
  }, [rows, sort, columns]);
  const grouped = visible.some((c) => c.group);
  const top: { label: string; span: number; group: boolean; col?: Column<T> }[] = [];
  for (const c of visible) {
    const last = top[top.length - 1];
    if (c.group && last?.group && last.label === c.group) last.span++;
    else top.push(c.group ? { label: c.group, span: 1, group: true } : { label: c.label, span: 1, group: false, col: c });
  }
  const header = (c: Column<T>, rowSpan?: number) => (
    <th
      key={c.key}
      rowSpan={rowSpan}
      className={`${c.num ? 'num' : ''}${c.sort ? ' sortable' : ''}`}
      onClick={() => c.sort && setSort((s) => ({ key: c.key, desc: s?.key === c.key ? !s.desc : true }))}
      aria-sort={sort?.key === c.key ? (sort.desc ? 'descending' : 'ascending') : undefined}
    >
      {c.label}
      {sort?.key === c.key ? (sort.desc ? ' ↓' : ' ↑') : ''}
    </th>
  );
  const width = visible.length + (expand ? 1 : 0);
  return (
    <table className="dsm-table">
      <thead>
        {grouped ? (
          <>
            <tr>
              {expand && <th rowSpan={2} />}
              {top.map((h) =>
                h.group ? (
                  <th key={`g-${h.label}`} colSpan={h.span} className="dsm-group-head">
                    {h.label}
                  </th>
                ) : (
                  header(h.col!, 2)
                ),
              )}
            </tr>
            <tr>{visible.filter((c) => c.group).map((c) => header(c))}</tr>
          </>
        ) : (
          <tr>
            {expand && <th />}
            {visible.map((c) => header(c))}
          </tr>
        )}
      </thead>
      <tbody>
        {sorted.length === 0 && (
          <tr>
            <td colSpan={width} className="dsm-empty-row">
              No matching results found
            </td>
          </tr>
        )}
        {sorted.map((r) => {
          const key = rowKey(r);
          const expanded = open === key;
          return (
            <Fragment key={key}>
              <tr className={expand ? `clickable${expanded ? ' selected' : ''}` : undefined} onClick={expand ? () => setOpen(expanded ? null : key) : undefined}>
                {expand && <td className="dsm-caret">{expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</td>}
                {visible.map((c) => (
                  <td key={c.key} className={c.num ? 'num' : undefined}>
                    {c.render(r)}
                  </td>
                ))}
              </tr>
              {expand && expanded && (
                <tr className="dsm-detail-row">
                  <td />
                  <td colSpan={visible.length}>{expand(r)}</td>
                </tr>
              )}
            </Fragment>
          );
        })}
      </tbody>
    </table>
  );
}

function ColumnSettings<T>({ columns, hidden, onChange }: { columns: Column<T>[]; hidden: Set<string>; onChange: (h: Set<string>) => void }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="dd" ref={ref}>
      <button className="btn btn-sm" onClick={() => setOpen((o) => !o)}>
        <Settings2 size={14} /> Column settings
      </button>
      {open && (
        <div className="dd-menu dd-right dsm-columns-menu">
          {columns.map((c, i) => (
            <label key={c.key} className="dsm-check">
              <input
                type="checkbox"
                checked={!hidden.has(c.key)}
                disabled={i === 0}
                onChange={(e) => {
                  const next = new Set(hidden);
                  if (e.target.checked) next.delete(c.key);
                  else next.add(c.key);
                  onChange(next);
                }}
              />
              {c.group ? `${c.group} · ${c.label}` : c.label}
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

// ================================================================ columns

function HealthCell({ c }: { c: KafkaCluster }) {
  const total = c.checksPassing + c.checksFailing;
  if (!total) return <span className="muted">No data</span>;
  return c.checksFailing ? (
    <span className="dsm-pill bad">
      <CircleX size={12} /> {c.checksFailing} failing
    </span>
  ) : (
    <span className="dsm-pill ok">
      <CircleCheck size={12} /> {c.checksPassing}/{total} passing
    </span>
  );
}

const clusterColumns = (go: (tab: Tab, cluster: string) => void): Column<KafkaCluster>[] => [
  {
    key: 'name',
    label: 'CLUSTER',
    render: (c) => (
      <span className="dsm-name">
        <IntegrationLogo logo="apachekafka" name="Kafka" size={16} /> {c.name}
      </span>
    ),
    sort: (c) => c.name,
  },
  {
    key: 'brokers',
    label: 'BROKERS',
    num: true,
    render: (c) => (
      <button className="link-btn" onClick={() => go('brokers', c.name)}>
        {n(c.brokers)}
      </button>
    ),
    sort: (c) => c.brokers ?? -1,
  },
  {
    key: 'topics',
    label: 'TOPICS',
    num: true,
    render: (c) => (
      <button className="link-btn" onClick={() => go('topics', c.name)}>
        {c.topics}
      </button>
    ),
    sort: (c) => c.topics,
  },
  { key: 'health', label: 'HEALTH', render: (c) => <HealthCell c={c} />, sort: (c) => c.checksFailing },
  {
    key: 'monitors',
    label: 'MONITORS',
    num: true,
    render: (c) => (c.monitors ? <Link to={`/monitors?q=${encodeURIComponent(`kafka ${c.name}`)}`}>{c.monitors}</Link> : <span className="muted">0</span>),
    sort: (c) => c.monitors,
  },
  { key: 'partitions', label: 'TOTAL', group: 'PARTITIONS', num: true, render: (c) => n(c.partitions), sort: (c) => c.partitions ?? -1 },
  {
    key: 'under',
    label: 'UNDER-REPLICATED',
    group: 'PARTITIONS',
    num: true,
    render: (c) => <span className={(c.underReplicated ?? 0) > 0 ? 'dsm-bad' : undefined}>{n(c.underReplicated)}</span>,
    sort: (c) => c.underReplicated ?? -1,
  },
  {
    key: 'offline',
    label: 'OFFLINE',
    group: 'PARTITIONS',
    num: true,
    render: (c) => <span className={(c.offline ?? 0) > 0 ? 'dsm-bad' : undefined}>{n(c.offline)}</span>,
    sort: (c) => c.offline ?? -1,
  },
  { key: 'groups', label: 'CONSUMER GROUPS', num: true, render: (c) => c.consumerGroups, sort: (c) => c.consumerGroups },
  { key: 'messages', label: 'MESSAGES IN', num: true, render: (c) => fmtRate(c.messagesIn), sort: (c) => c.messagesIn ?? -1 },
];

const topicColumns: Column<KafkaTopic>[] = [
  { key: 'name', label: 'TOPIC', render: (t) => <span className="dsm-name">{t.name}</span>, sort: (t) => t.name },
  { key: 'cluster', label: 'CLUSTER', render: (t) => t.cluster, sort: (t) => t.cluster },
  { key: 'partitions', label: 'PARTITIONS', num: true, render: (t) => n(t.partitions), sort: (t) => t.partitions ?? -1 },
  { key: 'replication', label: 'REPLICATION', num: true, render: (t) => n(t.replicationFactor), sort: (t) => t.replicationFactor ?? -1 },
  {
    key: 'under',
    label: 'UNDER-REPLICATED',
    num: true,
    render: (t) => <span className={(t.underReplicated ?? 0) > 0 ? 'dsm-bad' : undefined}>{n(t.underReplicated)}</span>,
    sort: (t) => t.underReplicated ?? -1,
  },
  { key: 'in', label: 'IN', group: 'MESSAGES', num: true, render: (t) => fmtRate(t.messagesIn), sort: (t) => t.messagesIn ?? -1 },
  { key: 'out', label: 'OUT', group: 'MESSAGES', num: true, render: (t) => fmtRate(t.messagesOut), sort: (t) => t.messagesOut ?? -1 },
  { key: 'bytesIn', label: 'IN', group: 'BYTES', num: true, render: (t) => fmtBytesRate(t.bytesIn), sort: (t) => t.bytesIn ?? -1 },
  { key: 'bytesOut', label: 'OUT', group: 'BYTES', num: true, render: (t) => fmtBytesRate(t.bytesOut), sort: (t) => t.bytesOut ?? -1 },
  { key: 'lag', label: 'CONSUMER LAG MAX', num: true, render: (t) => n(t.consumerLagMax), sort: (t) => t.consumerLagMax ?? -1 },
  { key: 'groups', label: 'CONSUMER GROUPS', render: (t) => t.consumerGroups.join(', ') || <span className="muted">—</span> },
];

const brokerColumns: Column<KafkaBroker>[] = [
  {
    key: 'id',
    label: 'BROKER',
    render: (b) => (
      <span className="dsm-name">
        {b.id}
        {b.controller && <span className="dsm-pill">controller</span>}
      </span>
    ),
    sort: (b) => b.id,
  },
  { key: 'cluster', label: 'CLUSTER', render: (b) => b.cluster, sort: (b) => b.cluster },
  { key: 'host', label: 'HOST', render: (b) => b.host ?? <span className="muted">—</span>, sort: (b) => b.host ?? '' },
  { key: 'partitions', label: 'PARTITIONS', num: true, render: (b) => n(b.partitions), sort: (b) => b.partitions ?? -1 },
  { key: 'leaders', label: 'LEADERS', num: true, render: (b) => n(b.leaders), sort: (b) => b.leaders ?? -1 },
  {
    key: 'under',
    label: 'UNDER-REPLICATED',
    num: true,
    render: (b) => <span className={(b.underReplicated ?? 0) > 0 ? 'dsm-bad' : undefined}>{n(b.underReplicated)}</span>,
    sort: (b) => b.underReplicated ?? -1,
  },
  { key: 'messages', label: 'MESSAGES IN', num: true, render: (b) => fmtRate(b.messagesIn), sort: (b) => b.messagesIn ?? -1 },
  { key: 'bytesIn', label: 'BYTES IN', num: true, render: (b) => fmtBytesRate(b.bytesIn), sort: (b) => b.bytesIn ?? -1 },
  { key: 'bytesOut', label: 'BYTES OUT', num: true, render: (b) => fmtBytesRate(b.bytesOut), sort: (b) => b.bytesOut ?? -1 },
];

const groupColumns: Column<KafkaConsumerGroup>[] = [
  { key: 'name', label: 'CONSUMER GROUP', render: (g) => <span className="dsm-name">{g.name}</span>, sort: (g) => g.name },
  { key: 'cluster', label: 'CLUSTER', render: (g) => g.cluster, sort: (g) => g.cluster },
  { key: 'state', label: 'STATE', render: (g) => g.state ?? <span className="muted">—</span>, sort: (g) => g.state ?? '' },
  { key: 'members', label: 'MEMBERS', num: true, render: (g) => n(g.members), sort: (g) => g.members ?? -1 },
  { key: 'topics', label: 'TOPICS', render: (g) => g.topics.map((t) => t.topic).join(', ') },
  { key: 'lag', label: 'MESSAGES', group: 'LAG', num: true, render: (g) => <span className={g.lagHigh ? 'dsm-bad' : undefined}>{fmtCount(g.lag)}</span>, sort: (g) => g.lag },
  { key: 'lagMax', label: 'MAX PARTITION', group: 'LAG', num: true, render: (g) => fmtCount(g.lagMax), sort: (g) => g.lagMax },
  { key: 'lagTime', label: 'TIME', group: 'LAG', num: true, render: (g) => <span className={g.lagHigh ? 'dsm-bad' : undefined}>{fmtLagSeconds(g.lagSeconds)}</span>, sort: (g) => g.lagSeconds ?? -1 },
  { key: 'rate', label: 'CONSUMED', num: true, render: (g) => fmtRate(g.consumeRate), sort: (g) => g.consumeRate ?? -1 },
];

const connectorColumns: Column<KafkaConnector>[] = [
  { key: 'name', label: 'CONNECTOR', render: (c) => <span className="dsm-name">{c.name}</span>, sort: (c) => c.name },
  { key: 'cluster', label: 'CLUSTER', render: (c) => c.cluster, sort: (c) => c.cluster },
  { key: 'type', label: 'TYPE', render: (c) => c.connectorType ?? <span className="muted">—</span>, sort: (c) => c.connectorType ?? '' },
  { key: 'class', label: 'CLASS', render: (c) => c.class ?? <span className="muted">—</span>, sort: (c) => c.class ?? '' },
  {
    key: 'state',
    label: 'STATE',
    render: (c) => (c.state ? <span className={`dsm-pill ${c.state === 'running' ? 'ok' : c.state === 'failed' ? 'bad' : ''}`}>{c.state}</span> : <span className="muted">—</span>),
    sort: (c) => c.state ?? '',
  },
  { key: 'tasks', label: 'TOTAL', group: 'TASKS', num: true, render: (c) => c.tasks, sort: (c) => c.tasks },
  { key: 'running', label: 'RUNNING', group: 'TASKS', num: true, render: (c) => c.running, sort: (c) => c.running },
  { key: 'failed', label: 'FAILED', group: 'TASKS', num: true, render: (c) => <span className={c.failed ? 'dsm-bad' : undefined}>{c.failed}</span>, sort: (c) => c.failed },
];

function GroupPartitions({ g }: { g: KafkaConsumerGroup }) {
  return (
    <table className="dsm-table compact">
      <thead>
        <tr>
          <th>TOPIC</th>
          <th className="num">PARTITION</th>
          <th className="num">BROKER OFFSET</th>
          <th className="num">CONSUMER OFFSET</th>
          <th className="num">LAG</th>
          <th className="num">LAG IN TIME</th>
        </tr>
      </thead>
      <tbody>
        {g.partitions.map((p) => (
          <tr key={`${p.topic}-${p.partition}`}>
            <td>{p.topic}</td>
            <td className="num">{p.partition}</td>
            <td className="num">{p.brokerOffset == null ? '—' : p.brokerOffset.toLocaleString()}</td>
            <td className="num">{p.consumerOffset == null ? '—' : p.consumerOffset.toLocaleString()}</td>
            <td className="num">{n(p.lag)}</td>
            <td className="num">{fmtLagSeconds(p.lagSeconds)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function useHidden(tab: Tab) {
  const key = `oc.dsm.kafka.columns.${tab}`;
  const [hidden, setHidden] = useState<Record<string, Set<string>>>({});
  const current = hidden[tab] ?? new Set<string>((store.get(key) ?? '').split(',').filter(Boolean));
  const set = (h: Set<string>) => {
    setHidden((x) => ({ ...x, [tab]: h }));
    store.set(key, [...h].join(','));
  };
  return [current, set] as const;
}

function CheckIcon({ status }: { status: KafkaHealthCheck['status'] }) {
  if (status === 'passing') return <CircleCheck size={14} className="dsm-ok" />;
  if (status === 'failing') return <CircleX size={14} className="dsm-bad" />;
  return <CircleDashed size={14} className="muted" />;
}

// ================================================================ page

/** Data Streams → Kafka Console: the health and inventory of Kafka clusters. */
export function KafkaConsolePage() {
  const q = useKafkaConsole();
  const [params, setParams] = useSearchParams();
  const tab = (TABS.find((t) => t.id === params.get('tab'))?.id ?? 'clusters') as Tab;
  const needle = params.get('q') ?? '';
  const cluster = params.get('cluster') ?? '';
  const filters = (params.get('filters') ?? '').split(',').filter((f): f is FilterId => FILTERS.some((x) => x.id === f));
  const [charts, setCharts] = useState(() => store.get('oc.dsm.kafka.charts') !== 'off');
  const [healthOpen, setHealthOpen] = useState(false);
  const [changesOpen, setChangesOpen] = useState(false);
  const [hidden, setHidden] = useHidden(tab);
  const set = (k: string, v: string | null) =>
    setParams(
      (p) => {
        const next = new URLSearchParams(p);
        if (v) next.set(k, v);
        else next.delete(k);
        return next;
      },
      { replace: true },
    );
  const k = q.data;
  const scoped = useMemo(() => (k ? scopeConsole(k, { needle, cluster, filters }) : null), [k, needle, cluster, filters.join(',')]); // eslint-disable-line react-hooks/exhaustive-deps
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!k || !scoped) return <Loading label="Loading Kafka clusters…" />;
  if (!k.enabled)
    return (
      <div className="dsm-page">
        <div className="dsm-kafka-empty">
          <IntegrationLogo logo="apachekafka" name="Kafka" size={40} />
          <h2>No Kafka metrics in this range</h2>
          <p className="muted">
            Enable the Datadog Agent's Kafka checks, or the OpenTelemetry Collector's Kafka receiver, and point them at this install. Clusters, topics, brokers and consumer groups appear here within a
            few minutes.
          </p>
          <Link className="btn btn-primary" to="/data-streams/setup?tab=kafka-agent">
            Set up a cluster
          </Link>
        </div>
      </div>
    );
  const go = (t: Tab, c: string) => setParams((p) => new URLSearchParams({ ...Object.fromEntries(p), tab: t, cluster: c }), { replace: true });
  const counts: Record<Tab, number> = {
    clusters: scoped.clusters.length,
    topics: scoped.topics.length,
    brokers: scoped.brokers.length,
    groups: scoped.groups.length,
    connectors: scoped.connectors.length,
  };
  const totals: Record<Tab, number> = { clusters: k.clusters.length, topics: k.topics.length, brokers: k.brokers.length, groups: k.consumerGroups.length, connectors: k.connectors.length };
  const clusterNames = new Set(scoped.clusters.map((c) => c.name));
  const checks = k.health.filter((h) => clusterNames.has(h.cluster));
  const passing = checks.filter((h) => h.status === 'passing').length;
  const failing = checks.filter((h) => h.status === 'failing').length;
  const changes = k.changes.filter((c) => !cluster || c.cluster === cluster);
  const chartsFor = k.charts.filter((c) => clusterNames.has(c.cluster));
  const noun = TABS.find((t) => t.id === tab)!.noun;
  const toggleCharts = () => {
    setCharts((c) => {
      store.set('oc.dsm.kafka.charts', c ? 'off' : 'on');
      return !c;
    });
  };
  const table = (() => {
    switch (tab) {
      case 'clusters': {
        const cols = clusterColumns(go);
        return { cols, node: <DataTable columns={cols} rows={scoped.clusters} rowKey={(c) => c.name} hidden={hidden} /> };
      }
      case 'topics':
        return { cols: topicColumns, node: <DataTable columns={topicColumns} rows={scoped.topics} rowKey={(t) => `${t.cluster}|${t.name}`} hidden={hidden} /> };
      case 'brokers':
        return { cols: brokerColumns, node: <DataTable columns={brokerColumns} rows={scoped.brokers} rowKey={(b) => `${b.cluster}|${b.id}`} hidden={hidden} /> };
      case 'groups':
        return {
          cols: groupColumns,
          node: <DataTable columns={groupColumns} rows={scoped.groups} rowKey={(g) => `${g.cluster}|${g.name}`} hidden={hidden} expand={(g) => <GroupPartitions g={g} />} />,
        };
      default:
        return {
          cols: connectorColumns,
          node:
            k.connectors.length === 0 ? (
              <p className="muted dsm-note">
                No Kafka Connect metrics: connectors appear when the Agent's kafka_consumer check has a Connect URL, or the Confluent Platform check reads the Connect workers.
              </p>
            ) : (
              <DataTable columns={connectorColumns} rows={scoped.connectors} rowKey={(c) => `${c.cluster}|${c.name}`} hidden={hidden} />
            ),
        };
    }
  })();
  return (
    <div className="dsm-page dsm-kafka">
      <div className="dsm-toolbar">
        <label className="dsm-search wide">
          <Search size={14} />
          <input value={needle} onChange={(e) => set('q', e.target.value || null)} placeholder="Filter clusters, topics, brokers, consumer groups, and connectors…" />
        </label>
        <Select
          label="Cluster"
          value={cluster}
          options={[{ value: '', label: 'All clusters' }, ...k.clusters.map((c) => ({ value: c.name, label: c.name }))]}
          onChange={(v) => set('cluster', v || null)}
        />
        <Menu
          triggerClassName="btn btn-sm"
          align="left"
          trigger={
            <>
              <Plus size={14} /> Filter
            </>
          }
          items={FILTERS.filter((f) => !filters.includes(f.id)).map((f) => ({ label: f.label, onClick: () => set('filters', [...filters, f.id].join(',')) }))}
        />
        {filters.map((f) => (
          <span key={f} className="dsm-filter-chip">
            {FILTERS.find((x) => x.id === f)!.label}
            <button className="icon-btn" title="Remove filter" onClick={() => set('filters', filters.filter((x) => x !== f).join(',') || null)}>
              <X size={12} />
            </button>
          </span>
        ))}
      </div>

      <div className="dsm-health">
        <button className="dsm-health-toggle" onClick={() => setHealthOpen((o) => !o)} aria-expanded={healthOpen}>
          <HeartPulse size={15} />
          <b>Health</b> · {passing} checks passing
          {failing > 0 && <span className="dsm-bad"> · {failing} failing</span>}
          {healthOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </button>
        <Link to="/monitors?q=kafka" className="dsm-health-link">
          View all monitors
        </Link>
      </div>
      {healthOpen && (
        <ul className="dsm-checks">
          {checks.map((h) => (
            <li key={`${h.cluster}|${h.name}`}>
              <CheckIcon status={h.status} />
              <b>{h.name}</b>
              <span className="muted">{h.cluster}</span>
              <span className="muted">{h.detail}</span>
            </li>
          ))}
        </ul>
      )}

      <div className="dsm-tabs-row">
        <nav className="dsm-subtabs" role="tablist">
          {TABS.map((t) => (
            <button key={t.id} role="tab" aria-selected={t.id === tab} className={`dsm-subtab${t.id === tab ? ' active' : ''}`} onClick={() => set('tab', t.id === 'clusters' ? null : t.id)}>
              {t.label} <span className="dsm-count">{counts[t.id]}</span>
            </button>
          ))}
        </nav>
        <div className="dsm-tabs-actions">
          <Link className="btn btn-sm" to="/data-streams/setup?tab=kafka-agent">
            <Plus size={14} /> Setup new cluster
          </Link>
          <label className="dsm-switch">
            <button type="button" className={`switch${charts ? ' on' : ''}`} role="switch" aria-checked={charts} onClick={toggleCharts} />
            Charts
          </label>
        </div>
      </div>

      <section className="dsm-changes">
        <button className="dsm-changes-toggle" onClick={() => setChangesOpen((o) => !o)} aria-expanded={changesOpen}>
          {changesOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          Changes <span className="dsm-count">{changes.length}</span>
        </button>
        {changesOpen &&
          (changes.length === 0 ? (
            <p className="muted dsm-note">No Kafka events in this range. The Agent's Kafka checks and events tagged with a cluster show up here.</p>
          ) : (
            <ul className="dsm-change-list">
              {changes.map((c) => (
                <li key={`${c.ts}-${c.title}`}>
                  <span className={`dsm-dot ${c.status}`} />
                  <span>{c.title}</span>
                  <span className="muted">{c.cluster ?? ''}</span>
                  <span className="muted">{timeAgo(c.ts)}</span>
                </li>
              ))}
            </ul>
          ))}
      </section>

      {charts && (
        <div className="dsm-chart-grid">
          <LineChart title="Incoming messages by cluster" fmt={fmtRate} series={chartsFor.map((c) => ({ name: c.cluster, points: c.messagesIn }))} />
          <LineChart title="Under-replicated partitions by cluster" fmt={fmtCount} series={chartsFor.map((c) => ({ name: c.cluster, points: c.underReplicated }))} />
          <LineChart title="Offline partitions by cluster" fmt={fmtCount} series={chartsFor.map((c) => ({ name: c.cluster, points: c.offline }))} />
        </div>
      )}

      <div className="dsm-table-head">
        <span>
          Showing <b>{counts[tab]}</b> of <b>{totals[tab]}</b> {noun}
        </span>
        <ColumnSettings columns={table.cols as Column<unknown>[]} hidden={hidden} onChange={setHidden} />
      </div>
      {table.node}
    </div>
  );
}
