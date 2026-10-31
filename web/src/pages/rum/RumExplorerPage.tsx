import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BarChart3, ChevronDown, ChevronUp, Download, List, PanelLeftClose, PanelLeftOpen, PlayCircle, Search, TrendingUp, X } from 'lucide-react';
import type { RumEventRow, RumEventType, RumFacet } from '../../api/rum/client';
import type { Facet } from '../../api/types';
import { Chart } from '../../components/charts/Chart';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact, timeAgo } from '../../lib/format';
import { formatQuery, parseQuery } from '../../lib/searchQuery';
import { token } from '../../theme/chartTheme';
import { fmtMsShort, fmtNs, useSelectedApplication } from './common';
import { useRumApplications, useRumEvents, useRumFacets } from './queries';

const minute = 60_000;

const TYPES: [RumEventType, string][] = [
  ['session', 'Sessions'],
  ['view', 'Views'],
  ['action', 'Actions'],
  ['error', 'Errors'],
  ['resource', 'Resources'],
  ['long_task', 'Long Tasks'],
  ['vital', 'Vitals'],
];

type Viz = 'list' | 'timeseries' | 'toplist';

interface Column {
  label: string;
  value: (r: RumEventRow) => React.ReactNode;
  num?: boolean;
}

const a = (r: RumEventRow, k: string) => r.attributes[k] ?? '';
const date = (t: number) => {
  const d = new Date(t);
  return `${d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })} ${d.toLocaleTimeString(undefined, { hour12: false })}.${String(d.getMilliseconds()).padStart(3, '0')}`;
};
/** A session with an event in the last 15 minutes is still going. */
const inProgress = (r: RumEventRow) => Date.now() - Number(a(r, 'session.last_event_at')) < 15 * minute;

const COLUMNS: Record<RumEventType, Column[]> = {
  session: [
    { label: 'SESSION TYPE', value: (r) => a(r, 'session.type') },
    { label: 'TIME SPENT', value: (r) => fmtMsShort(Number(a(r, 'session.time_spent'))), num: true },
    { label: 'VIEW COUNT', value: (r) => a(r, 'session.view.count'), num: true },
    { label: 'ERROR COUNT', value: (r) => a(r, 'session.error.count'), num: true },
    { label: 'ACTION COUNT', value: (r) => a(r, 'session.action.count'), num: true },
    { label: 'FRUSTRATION', value: (r) => <span className="rum-frustration">☹ {a(r, 'session.frustration.count')}</span> },
    { label: 'INITIAL VIEW NAME', value: (r) => a(r, 'session.initial_view.name') },
    { label: 'LAST VIEW NAME', value: (r) => (inProgress(r) ? <em className="muted">Session in progress</em> : a(r, 'session.last_view.name')) },
  ],
  view: [
    { label: 'VIEW NAME', value: (r) => a(r, 'view.name') },
    { label: 'LOADING TIME', value: (r) => fmtNs(a(r, 'view.loading_time')), num: true },
    { label: 'LCP', value: (r) => fmtNs(a(r, 'view.largest_contentful_paint')), num: true },
    { label: 'TIME SPENT', value: (r) => fmtNs(a(r, 'view.time_spent')), num: true },
    { label: 'ERRORS', value: (r) => a(r, 'view.error.count') || '0', num: true },
    { label: 'BROWSER', value: (r) => a(r, 'browser.name') || a(r, 'device.name') },
  ],
  action: [
    { label: 'ACTION TYPE', value: (r) => a(r, 'action.type') },
    { label: 'TARGET NAME', value: (r) => a(r, 'action.target.name') },
    { label: 'FRUSTRATION', value: (r) => a(r, 'action.frustration.type') },
    { label: 'VIEW NAME', value: (r) => a(r, 'view.name') },
  ],
  error: [
    { label: 'ERROR SOURCE', value: (r) => a(r, 'error.source') },
    { label: 'ERROR TYPE', value: (r) => a(r, 'error.type') },
    { label: 'MESSAGE', value: (r) => <span className="rum-msg">{a(r, 'error.message')}</span> },
    { label: 'VIEW NAME', value: (r) => a(r, 'view.name') },
  ],
  resource: [
    { label: 'METHOD', value: (r) => a(r, 'resource.method') },
    { label: 'URL', value: (r) => <span className="rum-msg">{a(r, 'resource.url')}</span> },
    { label: 'STATUS', value: (r) => a(r, 'resource.status_code'), num: true },
    { label: 'DURATION', value: (r) => fmtNs(a(r, 'resource.duration')), num: true },
    { label: 'TYPE', value: (r) => a(r, 'resource.type') },
  ],
  long_task: [
    { label: 'DURATION', value: (r) => fmtNs(a(r, 'long_task.duration')), num: true },
    { label: 'VIEW NAME', value: (r) => a(r, 'view.name') },
  ],
  vital: [
    { label: 'VITAL', value: (r) => a(r, 'vital.name') },
    { label: 'DURATION', value: (r) => fmtNs(a(r, 'vital.duration')), num: true },
    { label: 'VIEW NAME', value: (r) => a(r, 'view.name') },
  ],
  all: [
    { label: 'TYPE', value: (r) => r.type },
    { label: 'VIEW NAME', value: (r) => a(r, 'view.name') },
    { label: 'DETAIL', value: (r) => <span className="rum-msg">{a(r, 'error.message') || a(r, 'action.target.name') || a(r, 'resource.url')}</span> },
  ],
};

const FACET_GROUP: [RegExp, string][] = [
  [/^(application\.id|service|env|version|session\.type)$/, 'CORE'],
  [/^usr\./, 'USER'],
  [/^(device|browser|os)\./, 'DEVICE'],
  [/^geo\./, 'GEOLOCATION'],
  [/^view\./, 'VIEW'],
  [/^error\./, 'ERROR'],
  [/^action\./, 'ACTION'],
  [/^resource\./, 'RESOURCE'],
  [/^(long_task|vital)\./, 'PERFORMANCE'],
];
const LABEL: Record<string, string> = {
  'application.id': 'Application',
  service: 'Service',
  env: 'Env',
  version: 'Version',
  'session.type': 'Session Type',
  'usr.name': 'User Name',
  'usr.id': 'User Id',
  'usr.email': 'User Email',
  'device.type': 'Device Type',
  'device.name': 'Device Name',
  'device.model': 'Device Model',
  'browser.name': 'Browser Name',
  'os.name': 'OS Name',
  'geo.country': 'Country',
  'geo.city': 'City',
  'view.name': 'View Name',
  'view.url_path': 'View Path',
};
const label = (path: string) =>
  LABEL[path] ??
  path
    .split('.')
    .slice(1)
    .join(' ')
    .replace(/_/g, ' ')
    .replace(/\b\w/g, (c) => c.toUpperCase());
const facetKey = (path: string) => (['service', 'env', 'version'].includes(path) ? path : `@${path}`);

/** RUM → Explore: RUM events (sessions, views, actions, errors, resources, long tasks, vitals) with facets. */
export function RumExplorerPage() {
  const [params, setParams] = useSearchParams();
  const [selected] = useSelectedApplication();
  const apps = useRumApplications();
  const type = (TYPES.find(([t]) => t === params.get('type'))?.[0] ?? 'session') as RumEventType;
  const query = params.get('q') ?? '';
  const [draft, setDraft] = useState(query);
  useEffect(() => setDraft(query), [query]);
  const [viz, setViz] = useState<Viz>('list');
  const [controls, setControls] = useState(true);
  const [open, setOpen] = useState<RumEventRow | null>(null);
  const [topBy, setTopBy] = useState('view.name');
  const { range } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute }), [range.from, range.to]);
  const req = { range: r, query, eventType: type, applicationId: selected, limit: 100 };
  const events = useRumEvents(req);
  const facets = useRumFacets(req);
  const set = (k: string, v: string | null) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: true },
    );
  const appName = (id: string) => apps.data?.find((x) => x.id === id)?.name ?? id;
  const terms = useMemo(() => parseQuery(query), [query]);
  const facetList = useMemo<Facet[]>(
    () =>
      (facets.data ?? []).map((f: RumFacet) => ({
        key: facetKey(f.path),
        label: label(f.path),
        group: FACET_GROUP.find(([re]) => re.test(f.path))?.[1] ?? 'OTHER',
        values: f.values.map((v) => ({ value: v.value, count: v.count })),
      })),
    [facets.data],
  );
  const noun = TYPES.find(([t]) => t === type)?.[1] ?? 'Events';
  const data = events.data;
  const histogram = useMemo<EChartsOption>(
    () => ({
      grid: { left: 36, right: 12, top: 10, bottom: 22 },
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'time', min: r.from, max: r.to },
      yAxis: { type: 'value', minInterval: 1, splitNumber: 2 },
      series: [{ name: noun, type: 'bar', barMaxWidth: 14, itemStyle: { color: token('--chart-2') }, data: (data?.buckets ?? []).map((b) => [b.t, b.count]) }],
    }),
    [data, r.from, r.to, noun],
  );
  const top = (facets.data ?? []).find((f) => f.path === topBy);
  const download = () => {
    const rows = data?.rows ?? [];
    const cols = COLUMNS[type];
    const cell = (v: string) => (/[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v);
    const text = (n: React.ReactNode) => (typeof n === 'string' || typeof n === 'number' ? String(n) : '');
    const csv = [['DATE', ...cols.map((c) => c.label)], ...rows.map((row) => [new Date(row.timestamp).toISOString(), ...cols.map((c) => text(c.value(row)))])]
      .map((l) => l.map(cell).join(','))
      .join('\n');
    const link = document.createElement('a');
    link.href = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }));
    link.download = `rum-${type}.csv`;
    link.click();
    URL.revokeObjectURL(link.href);
  };

  return (
    <div className="rum-page rum-explorer">
      <div className="rum-subtabs">
        <span className="rum-subtab active">RUM Events</span>
        <span className="rum-subtab disabled" title="Feature flag tracking isn't available in RUM yet">
          Feature Flags
        </span>
        <span className="syn-grow" />
        <TimePicker compact />
      </div>
      <div className="rum-query">
        <label className="rum-in">
          <span>In</span>
          <select value={type} onChange={(e) => set('type', e.target.value === 'session' ? null : e.target.value)} aria-label="Event type">
            {TYPES.map(([t, l]) => (
              <option key={t} value={t}>
                {l}
              </option>
            ))}
          </select>
        </label>
        <form
          className="rum-search"
          onSubmit={(e) => {
            e.preventDefault();
            set('q', draft.trim() || null);
          }}
        >
          <Search size={15} />
          <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Filter your RUM events, e.g. @view.name:/checkout @browser.name:Chrome" aria-label="Search RUM events" />
        </form>
      </div>
      <div className="rum-viz">
        <span className="muted">Visualize as</span>
        {(
          [
            ['list', 'List', List],
            ['timeseries', 'Timeseries', TrendingUp],
            ['toplist', 'Top List', BarChart3],
          ] as [Viz, string, typeof List][]
        ).map(([v, l, Icon]) => (
          <button key={v} className={viz === v ? 'active' : ''} onClick={() => setViz(v)}>
            <Icon size={14} /> {l}
          </button>
        ))}
      </div>
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && (
          <FacetPanel
            facets={facetList}
            terms={terms}
            onChange={(t) => set('q', formatQuery(t) || null)}
            defaultOpen={['@session.type', '@device.type', '@browser.name', '@view.name', '@error.source', '@action.type', '@resource.type']}
            verified={[]}
            noun="facets"
            valueLabel={(k, v) => (k === '@application.id' ? appName(v) : v)}
          />
        )}
        <div className="syn-explorer-main">
          {viz === 'list' && (
            <section className="rum-histogram">
              <h4>{type === 'session' ? 'Session' : noun.replace(/s$/, '')} count</h4>
              <Chart option={histogram} height={110} />
            </section>
          )}
          <div className="syn-toolbar">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span className="syn-toolbar-sep" />
            <span>
              <b>{compact(data?.total ?? 0)}</b> {noun} found
            </span>
            <span className="syn-grow" />
            {viz === 'list' && (
              <button className="btn btn-sm" onClick={download} disabled={!data?.rows.length}>
                <Download size={13} /> Download as CSV
              </button>
            )}
            {viz === 'toplist' && (
              <label className="syn-select-field rum-inline">
                <span>Top by</span>
                <select value={topBy} onChange={(e) => setTopBy(e.target.value)}>
                  {(facets.data ?? []).map((f) => (
                    <option key={f.path} value={f.path}>
                      {label(f.path)}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
          {events.isPending ? (
            <Loading label={`Searching ${noun.toLowerCase()}…`} />
          ) : events.isError ? (
            <ErrorState error={events.error} onRetry={() => events.refetch()} />
          ) : viz === 'timeseries' ? (
            <Chart option={{ ...histogram, series: [{ ...(histogram.series as object[])[0], type: 'line', areaStyle: { opacity: 0.15 }, showSymbol: false }] } as EChartsOption} height={320} />
          ) : viz === 'toplist' ? (
            <div className="rum-bars rum-toplist">
              {(top?.values ?? []).map((v) => (
                <div key={v.value} className="rum-bar-row">
                  <span className="rum-bar-value">{compact(v.count)}</span>
                  <span className="rum-bar-track">
                    <i style={{ width: `${(v.count / Math.max(1, top!.values[0].count)) * 100}%` }} />
                    <span>{topBy === 'application.id' ? appName(v.value) : v.value}</span>
                  </span>
                </div>
              ))}
              {!top?.values.length && <p className="muted">No values in this time range.</p>}
            </div>
          ) : data!.rows.length === 0 ? (
            <div className="syn-empty syn-empty-search">
              <Search size={48} strokeWidth={1.2} />
              <h3>No {noun.toLowerCase()} found</h3>
              <p className="muted">{apps.data?.length ? 'Change the search, the facets or the time range.' : 'Create an application and add its SDK to your pages to collect RUM events.'}</p>
              {!apps.data?.length && (
                <Link className="btn btn-primary" to="/rum/applications?new=1">
                  New Application
                </Link>
              )}
            </div>
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table syn-table rum-table">
                <thead>
                  <tr>
                    <th className="rum-col-bar" />
                    <th>DATE</th>
                    {COLUMNS[type].map((c) => (
                      <th key={c.label} className={c.num ? 'syn-col-num' : ''}>
                        {c.label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {data!.rows.map((row) => (
                    <tr key={`${row.type}-${row.id}`} onClick={() => setOpen(row)} className={open?.id === row.id ? 'selected' : ''}>
                      <td className="rum-col-bar">
                        <i className={`rum-type-bar ${row.type}`} />
                      </td>
                      <td className="rum-date">{date(row.timestamp)}</td>
                      {COLUMNS[type].map((c) => (
                        <td key={c.label} className={c.num ? 'syn-col-num' : ''}>
                          {c.value(row)}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
              {data!.total > data!.rows.length && (
                <p className="muted rum-more">
                  The newest {data!.rows.length} of {compact(data!.total)}. Narrow the search or the time range to see others.
                </p>
              )}
            </div>
          )}
        </div>
      </div>
      {open && (
        <EventPanel
          row={open}
          appName={appName}
          onClose={() => setOpen(null)}
          siblings={data?.rows ?? []}
          onMove={setOpen}
          range={r}
          filterSession={(id) => {
            setOpen(null);
            setParams(
              (p) => {
                const n = new URLSearchParams(p);
                n.set('type', 'all');
                n.set('q', `@session.id:${id}`);
                return n;
              },
              { replace: true },
            );
          }}
        />
      )}
    </div>
  );
}

/** One event or session in a side panel: its fields, and for a session the events it is made of. */
function EventPanel({
  row,
  appName,
  onClose,
  siblings,
  onMove,
  range,
  filterSession,
}: {
  row: RumEventRow;
  appName: (id: string) => string;
  onClose: () => void;
  siblings: RumEventRow[];
  onMove: (r: RumEventRow) => void;
  range: { from: number; to: number };
  filterSession: (id: string) => void;
}) {
  const at = siblings.findIndex((s) => s.id === row.id && s.type === row.type);
  const session = row.type === 'session';
  const sid = row.attributes['session.id'];
  const events = useRumEvents({ range, query: `@session.id:${sid ?? '-'}`, eventType: 'all', applicationId: null, limit: 200, ascending: true });
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
      if (e.key === 'ArrowUp' && at > 0) onMove(siblings[at - 1]);
      if (e.key === 'ArrowDown' && at >= 0 && at < siblings.length - 1) onMove(siblings[at + 1]);
    };
    globalThis.addEventListener('keydown', onKey);
    return () => globalThis.removeEventListener('keydown', onKey);
  }, [at, siblings, onClose, onMove]);
  const title = session
    ? `Session · ${appName(row.attributes['application.id'] ?? '')}`
    : row.type === 'view'
      ? row.attributes['view.name']
      : row.type === 'error'
        ? row.attributes['error.message']
        : row.type === 'action'
          ? `${row.attributes['action.type'] ?? 'action'} on ${row.attributes['action.target.name'] ?? '—'}`
          : row.type === 'resource'
            ? `${row.attributes['resource.method'] ?? ''} ${row.attributes['resource.url'] ?? ''}`
            : row.type;
  const fields = Object.entries(row.attributes).sort(([x], [y]) => x.localeCompare(y));
  return createPortal(
    <div className="syn-drawer-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="syn-drawer rum-drawer" role="dialog" aria-label="RUM event">
        <div className="syn-drawer-head">
          <span className={`rum-type-pill ${row.type}`}>{row.type.replace('_', ' ').toUpperCase()}</span>
          <h2 className="rum-drawer-title">{title}</h2>
          <div className="syn-drawer-actions">
            <button className="icon-plain" title="Previous (↑)" disabled={at <= 0} onClick={() => onMove(siblings[at - 1])}>
              <ChevronUp size={18} />
            </button>
            <button className="icon-plain" title="Next (↓)" disabled={at < 0 || at >= siblings.length - 1} onClick={() => onMove(siblings[at + 1])}>
              <ChevronDown size={18} />
            </button>
            <button className="icon-plain" title="Close" onClick={onClose}>
              <X size={18} />
            </button>
          </div>
        </div>
        <div className="syn-drawer-body">
          <p className="muted">
            {new Date(row.timestamp).toLocaleString()} ({timeAgo(row.timestamp)}) · {row.service || 'no service'}
            {row.attributes['usr.name'] ? ` · ${row.attributes['usr.name']}` : ''}
            {row.attributes['browser.name'] ? ` · ${row.attributes['browser.name']}` : ''}
            {row.attributes['os.name'] ? ` on ${row.attributes['os.name']}` : ''}
          </p>
          {session && (
            <div className="rum-session-facts">
              {(
                [
                  ['Time spent', fmtMsShort(Number(row.attributes['session.time_spent']))],
                  ['Views', row.attributes['session.view.count']],
                  ['Actions', row.attributes['session.action.count']],
                  ['Errors', row.attributes['session.error.count']],
                  ['Frustrations', row.attributes['session.frustration.count']],
                ] as [string, string][]
              ).map(([k, v]) => (
                <div key={k}>
                  <span className="muted">{k}</span>
                  <b>{v}</b>
                </div>
              ))}
              {row.attributes['session.has_replay'] === 'true' && (
                <Link className="btn rum-replay" to={`/rum/replay/sessions/${sid}?ts=${row.timestamp}`}>
                  <PlayCircle size={16} /> Watch the replay
                </Link>
              )}
            </div>
          )}
          {sid && (
            <section className="syn-run-section">
              <h4 className="rum-panel-h">
                {session ? 'Events of the session' : 'Session'}
                <button className="link-btn" onClick={() => filterSession(sid)}>
                  Show in the explorer
                </button>
              </h4>
              {events.isPending ? (
                <Loading label="Loading the session…" />
              ) : (
                <div className="rum-timeline">
                  {(events.data?.rows ?? []).map((e, i) => (
                    <button key={`${e.id}-${i}`} className={`rum-timeline-row${!session && e.id === row.id ? ' current' : ''}`} onClick={() => onMove(e)}>
                      <i className={`rum-type-bar ${e.type}`} />
                      <span className="muted">{new Date(e.timestamp).toLocaleTimeString(undefined, { hour12: false })}</span>
                      <b>{e.type.replace('_', ' ')}</b>
                      <span className="rum-msg">{e.attributes['error.message'] ?? e.attributes['action.target.name'] ?? e.attributes['resource.url'] ?? e.attributes['view.name'] ?? ''}</span>
                    </button>
                  ))}
                </div>
              )}
            </section>
          )}
          <section className="syn-run-section">
            <h4>Attributes</h4>
            <table className="syn-kv-table">
              <tbody>
                {fields.map(([k, v]) => (
                  <tr key={k}>
                    <td className="mono syn-col-key">{k}</td>
                    <td className="mono syn-wrap">{k === 'application.id' ? `${v} (${appName(v)})` : v}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
        </div>
      </aside>
    </div>,
    document.body,
  );
}
