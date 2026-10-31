import { useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import {
  BarChart3,
  ChartPie,
  ChevronDown,
  Code2,
  Container,
  Copy,
  Download,
  ExternalLink,
  FileText,
  Grid2x2,
  LineChart,
  List,
  ListOrdered,
  Network,
  Plus,
  Search,
  Server,
  ShieldCheck,
  Table2,
  Cpu,
  X,
} from 'lucide-react';
import type { WpEvent } from '../../../api/workloadProtection/client';
import type { Facet } from '../../../api/types';
import { BrandLogo, hasBrandLogo } from '../../../components/brand/BrandLogo';
import { Chart } from '../../../components/charts/Chart';
import { FacetPanel } from '../../../components/facets/FacetPanel';
import { TimePicker } from '../../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../../components/shell/TimeContext';
import { Menu } from '../../../components/ui/Dropdown';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact } from '../../../lib/format';
import { formatQuery, onlyValue, parseQuery, type Term } from '../../../lib/searchQuery';
import { token } from '../../../theme/chartTheme';
import { downloadCsv, ExplorerToolbar } from '../appsec/common';
import { useWpEvents } from './exploreQueries';
import './explore.css';

const MIN = 60_000;
const BASE = '/security/workload-protection';
const DOCS = 'https://github.com/observecraft/observecraft/blob/main/docs/sending-data.md#workload-protection';
const VIEWS_KEY = 'oc.wp.events.views';
const CATEGORY_COLOR: Record<string, string> = { file: '--chart-1', process: '--chart-2', network: '--chart-3', kernel: '--chart-4' };
const VISUALIZE: [string, typeof List, boolean][] = [
  ['List', List, true],
  ['Timeseries', LineChart, true],
  ['Top List', ListOrdered, true],
  ['Bar Chart', BarChart3, false],
  ['Table', Table2, false],
  ['Pie Chart', ChartPie, false],
  ['Tree Map', Grid2x2, false],
];
type Viz = 'List' | 'Timeseries' | 'Top List';

const dateTime = (ms: number) => new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });

/** The category of an event as the series key it ("Process Activity" → process). */
const categoryKey = (c: string) => c.toLowerCase().split(' ')[0];

function CategoryIcon({ category }: { category: string }) {
  const k = categoryKey(category);
  return k === 'file' ? <FileText size={13} /> : k === 'process' ? <Cpu size={13} /> : k === 'network' ? <Network size={13} /> : <ShieldCheck size={13} />;
}

interface SavedView {
  name: string;
  query: string;
}

function loadViews(): SavedView[] {
  try {
    return JSON.parse(localStorage.getItem(VIEWS_KEY) ?? '[]') as SavedView[];
  } catch {
    return [];
  }
}

function storeViews(views: SavedView[]) {
  try {
    localStorage.setItem(VIEWS_KEY, JSON.stringify(views));
  } catch {
    // Saved views are a convenience of this browser.
  }
}

/** Workload Protection → Investigate: the events the Agents captured with the agent rules. */
export function AgentEventsPage() {
  return (
    <TimeProvider initial="1d">
      <AgentEvents />
    </TimeProvider>
  );
}

function AgentEvents() {
  const { range } = useTime();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const setQuery = (v: string) => setParams((p) => (v ? p.set('q', v) : p.delete('q'), p), { replace: true });
  const [draft, setDraft] = useState(query);
  const [controls, setControls] = useState(true);
  const [viz, setViz] = useState<Viz>('List');
  const [groupInto, setGroupInto] = useState<'fields' | 'patterns'>('fields');
  const [topBy, setTopBy] = useState('@agent.rule_id');
  const [open, setOpen] = useState<WpEvent | null>(null);
  const [views, setViews] = useState<SavedView[]>(loadViews);
  const r = useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
  const q = useWpEvents(r, query, 200);
  const d = q.data;
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets: Facet[] = (d?.facets ?? []).map((f) => ({ key: f.key, label: f.label, group: 'CORE', values: f.values.map((v) => ({ value: v.name, count: v.count })) }));
  const apply = (v: string) => {
    setDraft(v);
    setQuery(v);
  };
  const save = () => {
    const name = window.prompt('Name of the view', query || 'All agent events');
    if (!name) return;
    const next = [...views.filter((v) => v.name !== name), { name, query }];
    setViews(next);
    storeViews(next);
  };
  const series = d?.series;
  const chart = useMemo<EChartsOption>(
    () => ({
      grid: { left: 40, right: 12, top: 14, bottom: 40 },
      tooltip: { trigger: 'axis' },
      legend: { bottom: 0, icon: 'rect', itemWidth: 9, itemHeight: 9 },
      xAxis: { type: 'time', min: r.from, max: r.to },
      yAxis: { type: 'value', minInterval: 1, splitNumber: 4 },
      series: (series?.groups ?? []).map((g) => ({
        name: g.name,
        type: viz === 'Timeseries' ? 'line' : 'bar',
        stack: 'a',
        showSymbol: false,
        areaStyle: viz === 'Timeseries' ? { opacity: 0.15 } : undefined,
        barMaxWidth: 10,
        itemStyle: { color: token(CATEGORY_COLOR[g.name] ?? '--chart-5') },
        data: (series?.times ?? []).map((t, i) => [t, g.values[i] ?? 0]),
      })),
    }),
    [series, r, viz],
  );
  const patterns = useMemo(() => {
    const m = new Map<string, { title: string; rule: string; category: string; n: number; hosts: Set<string>; last: number }>();
    for (const e of d?.events ?? []) {
      const k = `${e.ruleId}|${e.title}`;
      const p = m.get(k) ?? { title: e.title, rule: e.ruleId, category: e.category, n: 0, hosts: new Set<string>(), last: 0 };
      p.n++;
      p.hosts.add(e.host);
      p.last = Math.max(p.last, e.timestamp);
      m.set(k, p);
    }
    return [...m.values()].sort((a, b) => b.n - a.n);
  }, [d]);
  const top = facets.find((f) => f.key === topBy) ?? facets[0];
  const maxTop = Math.max(1, ...(top?.values ?? []).map((v) => v.count));
  const empty = !q.isLoading && !q.isError && !d?.total && !query;
  return (
    <div className="sec-explorer inv-page wpx-page">
      <div className="wpx-head">
        <Menu
          align="left"
          triggerClassName="link-btn wpx-views"
          trigger={
            <>
              <List size={14} /> Views <ChevronDown size={12} />
            </>
          }
          items={
            views.length
              ? [{ label: 'All agent events', onClick: () => apply('') }, ...views.map((v) => ({ label: v.name, onClick: () => apply(v.query) }))]
              : [{ label: 'No saved views yet', onClick: () => undefined }]
          }
        />
        <span className="inv-sep" />
        <b className="wpx-title">Agent Events Explorer</b>
        <button className="btn btn-sm" onClick={save} title="Saves this search in this browser">
          <Plus size={13} /> Save
        </button>
        <span className="syn-grow" />
        <TimePicker compact />
      </div>
      <div className="inv-top">
        <form
          className="inv-search"
          onSubmit={(e) => {
            e.preventDefault();
            setQuery(draft.trim());
          }}
        >
          <Search size={15} />
          <input placeholder="Filter agent events" value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => setQuery(draft.trim())} />
          <span className="inv-search-code" title="Search syntax: key:value, @attribute:value, -negation, free text">
            <Code2 size={14} />
          </span>
        </form>
        <span className="inv-in">
          <span>In</span>
          <b>Security events</b>
        </span>
      </div>
      <div className="wpx-rows">
        <span className="inv-seg">
          <span className="inv-seg-label">Group into</span>
          <button className={groupInto === 'fields' ? 'active' : ''} onClick={() => setGroupInto('fields')}>
            Fields
          </button>
          <button className={groupInto === 'patterns' ? 'active' : ''} onClick={() => (setGroupInto('patterns'), setViz('List'))}>
            Patterns
          </button>
        </span>
        <span className="inv-seg">
          <span className="inv-seg-label">Visualize as</span>
          {VISUALIZE.map(([label, Icon, on]) => (
            <button
              key={label}
              className={viz === label && groupInto === 'fields' ? 'active' : ''}
              disabled={!on || groupInto === 'patterns'}
              title={on ? undefined : 'Not available for agent events yet'}
              onClick={() => setViz(label as Viz)}
            >
              <Icon size={13} /> {label}
            </button>
          ))}
        </span>
        {viz === 'Top List' && groupInto === 'fields' && (
          <label className="fi-select wpx-by">
            <span>by</span>
            <select value={top?.key ?? ''} onChange={(e) => setTopBy(e.target.value)}>
              {facets.map((f) => (
                <option key={f.key} value={f.key}>
                  {f.label}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>
      {empty ? (
        <div className="wpx-empty">
          <ShieldCheck size={44} />
          <h3>No agent events</h3>
          <p className="muted">No hosts are reporting Workload Protection events yet. Enable runtime security on your Agents; the events they capture will show up here.</p>
          <a className="btn btn-primary" href={DOCS} target="_blank" rel="noreferrer">
            Set Up Workload Protection on Your Agents <ExternalLink size={13} />
          </a>
        </div>
      ) : (
        <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
          {controls && (
            <FacetPanel
              facets={facets}
              terms={terms}
              onChange={(t: Term[]) => apply(formatQuery(t))}
              defaultOpen={['@evt.category', '@agent.rule_id', 'host']}
              verified={[]}
              noun="facets"
              valueSearch
            />
          )}
          <div className="syn-explorer-main">
            <ExplorerToolbar
              controls={controls}
              onToggle={() => setControls((c) => !c)}
              right={
                <button
                  className="btn btn-sm"
                  disabled={!d?.events.length}
                  onClick={() =>
                    downloadCsv(
                      'agent-events.csv',
                      ['timestamp', 'host', 'category', 'event', 'rule', 'title', 'process', 'args', 'file', 'user', 'container', 'image'],
                      (d?.events ?? []).map((e) => [
                        new Date(e.timestamp).toISOString(),
                        e.host,
                        e.category,
                        e.event,
                        e.ruleId,
                        e.title,
                        e.process,
                        e.processArgs,
                        e.file,
                        e.user,
                        e.containerName,
                        e.image,
                      ]),
                    )
                  }
                >
                  <Download size={13} /> Export
                </button>
              }
            >
              <span>
                <b>{compact(d?.total ?? 0)}</b> {groupInto === 'patterns' ? `events in ${patterns.length} patterns` : 'events found'}
              </span>
            </ExplorerToolbar>
            {q.isLoading ? (
              <Loading />
            ) : q.isError ? (
              <ErrorState error={q.error} onRetry={() => q.refetch()} />
            ) : !d!.events.length ? (
              <div className="fi-empty">
                <Search size={40} />
                <p>No matching results found</p>
                <span className="muted">Change the search, the facets or the time range.</span>
              </div>
            ) : groupInto === 'patterns' ? (
              <table className="mon-table syn-table inv-table">
                <thead>
                  <tr>
                    <th className="num">COUNT</th>
                    <th>PATTERN</th>
                    <th>AGENT RULE</th>
                    <th className="num">HOSTS</th>
                    <th>LAST SEEN</th>
                  </tr>
                </thead>
                <tbody>
                  {patterns.map((p) => (
                    <tr key={`${p.rule}${p.title}`} onClick={() => apply(`@agent.rule_id:${p.rule}`)}>
                      <td className="num">
                        <b>{compact(p.n)}</b>
                      </td>
                      <td>
                        <span className={`wpx-cat c-${categoryKey(p.category)}`}>
                          <CategoryIcon category={p.category} />
                        </span>{' '}
                        {p.title}
                      </td>
                      <td className="mono">{p.rule}</td>
                      <td className="num">{p.hosts.size}</td>
                      <td className="nowrap">{dateTime(p.last)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : viz === 'Timeseries' ? (
              <div className="wpx-chart">
                <Chart option={chart} height={360} />
              </div>
            ) : viz === 'Top List' ? (
              <ul className="inv-toplist wpx-toplist">
                {(top?.values ?? []).map((v) => (
                  <li key={v.value} onClick={() => apply(formatQuery(onlyValue(terms, top!.key, v.value)))}>
                    <b>{compact(v.count)}</b>
                    <span className="inv-toplist-bar">
                      <i style={{ width: `${(v.count / maxTop) * 100}%` }} />
                      <span>{v.value}</span>
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <>
                <div className="wpx-mini">
                  <Chart option={chart} height={130} />
                </div>
                <table className="mon-table syn-table inv-table wpx-table">
                  <thead>
                    <tr>
                      <th>DATE</th>
                      <th>HOST</th>
                      <th>CATEGORY</th>
                      <th>AGENT RULE</th>
                      <th>CONTENT</th>
                      <th>USER</th>
                      <th>CONTAINER</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d!.events.map((e) => (
                      <tr key={e.id} onClick={() => setOpen(e)} className={open?.id === e.id ? 'selected' : ''}>
                        <td className="nowrap">{dateTime(e.timestamp)}</td>
                        <td className="nowrap">
                          <span className="wpx-host">
                            <Server size={12} /> {e.host}
                          </span>
                        </td>
                        <td className="nowrap">
                          <span className={`wpx-cat c-${categoryKey(e.category)}`}>
                            <CategoryIcon category={e.category} /> {e.event}
                          </span>
                        </td>
                        <td className="mono nowrap">{e.ruleId}</td>
                        <td className="wpx-content">
                          <b>{e.title}</b>
                          <span className="mono muted">
                            {e.process} {e.processArgs}
                            {e.file && ` → ${e.file}`}
                          </span>
                        </td>
                        <td className="nowrap">{e.user}</td>
                        <td className="nowrap">
                          {e.image && (
                            <span className="fi-chip" title={e.containerName}>
                              <Container size={11} /> {e.image}
                            </span>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                {d!.total > d!.events.length && (
                  <p className="muted wpx-more">
                    Showing the newest {d!.events.length} of {compact(d!.total)} events; narrow the search to see the others.
                  </p>
                )}
              </>
            )}
          </div>
        </div>
      )}
      {open && createPortal(<EventPanel e={open} onClose={() => setOpen(null)} onFilter={(k, v) => (apply(`${k}:${v}`), setOpen(null))} />, document.body)}
    </div>
  );
}

type Json = string | number | boolean | null | Json[] | { [k: string]: Json };

/** Flattens the event attributes to dotted keys. */
function flatten(o: Json, prefix = '', out: [string, string][] = []): [string, string][] {
  if (o && typeof o === 'object' && !Array.isArray(o)) {
    for (const [k, v] of Object.entries(o)) flatten(v, prefix ? `${prefix}.${k}` : k, out);
  } else if (Array.isArray(o)) {
    if (o.every((x) => typeof x !== 'object' || x === null)) out.push([prefix, o.join(' ')]);
    else o.forEach((x, i) => flatten(x, `${prefix}[${i}]`, out));
  } else if (o !== undefined) {
    out.push([prefix, String(o)]);
  }
  return out;
}

interface Proc {
  name: string;
  path: string;
  args: string;
  user: string;
}

/** The process and its ancestors, the closest first: from `process.ancestors` when the Agent sent them, else the parent. */
function processTree(attrs: Record<string, Json>, e: WpEvent): Proc[] {
  const p = (attrs.process ?? {}) as Record<string, Json>;
  const of = (x: Record<string, Json>): Proc => {
    const exe = (x.executable ?? {}) as Record<string, Json>;
    const args = Array.isArray(x.args) ? x.args.join(' ') : String(x.args ?? '');
    return { name: String(x.comm ?? exe.name ?? ''), path: String(exe.path ?? ''), args, user: String(x.user ?? '') };
  };
  const self: Proc = { ...of(p), path: e.process, args: e.processArgs, user: e.user };
  const ancestors = Array.isArray(p.ancestors) ? (p.ancestors as Record<string, Json>[]).map(of) : p.parent ? [of(p.parent as Record<string, Json>)] : [];
  return [self, ...ancestors];
}

/** One agent event: the process tree, the matched rule and every attribute the Agent sent. */
function EventPanel({ e, onClose, onFilter }: { e: WpEvent; onClose: () => void; onFilter: (key: string, value: string) => void }) {
  const [tab, setTab] = useState<'overview' | 'json'>('overview');
  const attrs = (e.attributes ?? {}) as Record<string, Json>;
  const agent = (attrs.agent ?? {}) as Record<string, Json>;
  const os = String(agent.os ?? '');
  const tree = processTree(attrs, e);
  const rows = flatten(attrs as Json);
  const json = JSON.stringify(attrs, null, 2);
  return (
    <div className="sec-panel-backdrop" onMouseDown={(ev) => ev.target === ev.currentTarget && onClose()}>
      <aside className="sec-panel inv-drawer wpx-drawer" role="dialog" aria-label="Agent event">
        <header className="inv-drawer-head">
          <div className="inv-crumbs">
            <span className={`wpx-cat c-${categoryKey(e.category)}`}>
              <CategoryIcon category={e.category} /> {e.category}
            </span>
            <b>{e.title}</b>
          </div>
          <span className="syn-grow" />
          <button className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <div className="inv-drawer-sub">
          <span className="muted">{dateTime(e.timestamp)}</span>
          <span className="wpx-host">
            {hasBrandLogo(os) ? <BrandLogo name={os} size={13} title={os} /> : <Server size={12} />} {e.host}
          </span>
          {e.env && <span className="tag-chip">env:{e.env}</span>}
          {e.image && (
            <span className="fi-chip">
              <Container size={11} /> {e.image}
            </span>
          )}
          <span className="tag-chip">evt.name:{e.event}</span>
          <span className="tag-chip">outcome:{e.outcome}</span>
        </div>
        <nav className="inv-tabs">
          <button className={tab === 'overview' ? 'active' : ''} onClick={() => setTab('overview')}>
            Overview
          </button>
          <button className={tab === 'json' ? 'active' : ''} onClick={() => setTab('json')}>
            JSON
          </button>
        </nav>
        <div className="inv-drawer-body">
          {tab === 'overview' ? (
            <>
              <section className="wpx-section">
                <h4>AGENT RULE</h4>
                <div className="wpx-rule">
                  <span className="mono">{e.ruleId}</span>
                  {agent.policy_name ? <span className="tag-chip">policy:{String(agent.policy_name)}</span> : null}
                  <span className="syn-grow" />
                  <button className="link-btn" onClick={() => onFilter('@agent.rule_id', e.ruleId)}>
                    Show all events of this rule
                  </button>
                  <Link className="link-btn" to={`${BASE}/policies`}>
                    Agent rules
                  </Link>
                </div>
              </section>
              <section className="wpx-section">
                <h4>PROCESS TREE</h4>
                <ol className="wpx-tree">
                  {[...tree].reverse().map((p, i, all) => (
                    <li key={i} className={i === all.length - 1 ? 'self' : ''} style={{ paddingLeft: i * 16 }}>
                      <Cpu size={13} />
                      <b className="mono">{p.name || p.path.split('/').pop()}</b>
                      {p.path && <span className="mono muted">{p.path}</span>}
                      {p.args && <span className="mono wpx-args">{p.args}</span>}
                      {p.user && <span className="tag-chip">user:{p.user}</span>}
                    </li>
                  ))}
                </ol>
              </section>
              {(e.file || e.containerId) && (
                <section className="wpx-section">
                  <h4>TARGET</h4>
                  <dl className="inv-kv">
                    {e.file && (
                      <>
                        <dt>{categoryKey(e.category) === 'network' ? 'Domain' : 'File'}</dt>
                        <dd className="mono">{e.file}</dd>
                      </>
                    )}
                    {e.containerId && (
                      <>
                        <dt>Container</dt>
                        <dd className="mono">
                          {e.containerName} <span className="muted">{e.containerId.slice(0, 12)}</span>
                        </dd>
                      </>
                    )}
                  </dl>
                </section>
              )}
              <section className="wpx-section">
                <h4>ATTRIBUTES</h4>
                <table className="wpx-attrs">
                  <tbody>
                    {rows.map(([k, v]) => (
                      <tr key={k}>
                        <td className="mono muted">{k}</td>
                        <td className="mono">{v}</td>
                        <td>
                          {!k.includes('[') && (
                            <button className="fi-icon-btn" title={`Filter on @${k}`} aria-label={`Filter on ${k}`} onClick={() => onFilter(`@${k}`, /\s/.test(v) ? `"${v}"` : v)}>
                              <Search size={12} />
                            </button>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </section>
            </>
          ) : (
            <div className="wpx-json">
              <button className="btn btn-sm" onClick={() => navigator.clipboard?.writeText(json)}>
                <Copy size={13} /> Copy
              </button>
              <pre>{json}</pre>
            </div>
          )}
        </div>
      </aside>
    </div>
  );
}
