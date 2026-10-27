import { useCallback, useEffect, useMemo, useState } from 'react';
import type { EChartsOption } from 'echarts';
import { ChevronDown, Code2, Copy, EllipsisVertical, LayoutDashboard, LayoutGrid, LineChart, List, ListOrdered, PanelLeft, PanelLeftClose, Pin, Settings, Table2, X } from 'lucide-react';
import { searchEvents, type EventItem, type EventPage } from '../../api/events/client';
import { Chart } from '../../components/charts/Chart';
import { Markdown } from '../dashboards/widgets/markdown';
import { eventMarkdown, eventSnippet } from '../../lib/format';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { compact, formatLongDate, hhmm, timeAgo } from '../../lib/format';
import { addTerm, formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { token } from '../../theme/chartTheme';
import { SourceIcon, StatusBadge, STATUS_TOKENS } from './eventVisuals';

type Viz = 'list' | 'timeseries' | 'toplist' | 'table' | 'patterns';

const VIZ: { key: Viz; label: string; icon: typeof List }[] = [
  { key: 'list', label: 'List', icon: List },
  { key: 'timeseries', label: 'Timeseries', icon: LineChart },
  { key: 'toplist', label: 'Top List', icon: ListOrdered },
  { key: 'table', label: 'Table', icon: Table2 },
  { key: 'patterns', label: 'Patterns', icon: LayoutGrid },
];

const STATUSES = ['error', 'warning', 'info', 'success'];
/** Tag chips shown on a row before "+N". */
const TAGS_SHOWN = 4;

function Histogram({ page, height }: { page: EventPage | null; height: number }) {
  const option = useMemo<EChartsOption>(() => {
    const buckets = page?.histogram ?? [];
    return {
      grid: { left: 36, right: 12, top: 8, bottom: 20 },
      tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
      xAxis: {
        type: 'category',
        data: buckets.map((b) => String(b.t)),
        axisLabel: { formatter: (v: string) => hhmm(Number(v)), interval: Math.max(0, Math.floor(buckets.length / 14) - 1) },
      },
      yAxis: { type: 'value', minInterval: 1, splitNumber: 1, axisLabel: { formatter: (v: number) => compact(v).replace('K', 'k') } },
      series: STATUSES.map((s) => ({
        name: s,
        type: 'bar',
        stack: 'status',
        barCategoryGap: '25%',
        barMaxWidth: 12,
        color: token(STATUS_TOKENS[s]),
        data: buckets.map((b) => b.counts[s] ?? 0),
      })),
    };
  }, [page]);
  return <Chart option={option} height={height} />;
}

/** Event titles with numbers and ids masked, so repeats of one event group together. */
function pattern(title: string): string {
  return title.replace(/[0-9a-f]{6,}/gi, '*').replace(/\d+(\.\d+)*/g, '*');
}

function Aggregate({ viz, page }: { viz: Viz; page: EventPage }) {
  if (viz === 'toplist' || viz === 'table') {
    const source = page.facets.find((f) => f.key === 'source')?.values ?? [];
    const max = Math.max(1, ...source.map((v) => v.count));
    return (
      <table className="ev-agg">
        <thead>
          <tr>
            <th>SOURCE</th>
            <th className="num">COUNT</th>
          </tr>
        </thead>
        <tbody>
          {source.map((v) => (
            <tr key={v.value}>
              <td>
                <SourceIcon source={v.value} size={14} /> {v.value}
              </td>
              <td className="num">
                {viz === 'toplist' && <span className="ev-bar" style={{ width: `${(v.count / max) * 120}px` }} />}
                {v.count.toLocaleString()}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    );
  }
  const groups = new Map<string, { count: number; sample: EventItem }>();
  for (const e of page.items) {
    const k = pattern(e.title);
    const g = groups.get(k);
    if (g) g.count += 1;
    else groups.set(k, { count: 1, sample: e });
  }
  return (
    <table className="ev-agg">
      <thead>
        <tr>
          <th>PATTERN</th>
          <th>SOURCE</th>
          <th className="num">COUNT</th>
        </tr>
      </thead>
      <tbody>
        {[...groups]
          .sort((a, b) => b[1].count - a[1].count)
          .map(([k, g]) => (
            <tr key={k}>
              <td>{k}</td>
              <td>{g.sample.source}</td>
              <td className="num">{g.count}</td>
            </tr>
          ))}
      </tbody>
    </table>
  );
}

function EventDetail({ event, onClose, onFilter }: { event: EventItem; onClose: () => void; onFilter: (k: string, v: string) => void }) {
  return (
    <aside className="ev-detail">
      <header>
        <SourceIcon source={event.source} />
        <StatusBadge status={event.alertType} />
        <span className="ev-detail-date">{formatLongDate(event.timestamp)}</span>
        <button className="ev-detail-close" onClick={onClose} title="Close">
          <X size={16} />
        </button>
      </header>
      <h2>{event.title}</h2>
      {event.text && (
        <div className="ev-detail-text">
          <Markdown source={eventMarkdown(event.text)} />
        </div>
      )}
      <dl>
        <dt>Source</dt>
        <dd>
          <button onClick={() => onFilter('source', event.source)}>{event.source || '—'}</button>
        </dd>
        <dt>Host</dt>
        <dd>{event.host ? <button onClick={() => onFilter('host', event.host)}>{event.host}</button> : '—'}</dd>
        <dt>Priority</dt>
        <dd>{event.priority}</dd>
        {event.aggregationKey && (
          <>
            <dt>Aggregation key</dt>
            <dd>{event.aggregationKey}</dd>
          </>
        )}
      </dl>
      {event.tags.length > 0 && (
        <div className="ev-detail-tags">
          {event.tags.map((t) => {
            const i = t.indexOf(':');
            return (
              <button key={t} className="ev-tag" onClick={() => i > 0 && onFilter(t.slice(0, i), t.slice(i + 1))}>
                {t}
              </button>
            );
          })}
        </div>
      )}
    </aside>
  );
}

/** Event Management → All Events: every event in the range, with facets and a status histogram. */
export function EventExplorer() {
  const { range } = useTime();
  const [query, setQuery] = useState('');
  const [viz, setViz] = useState<Viz>('list');
  const [showFacets, setShowFacets] = useState(true);
  const [page, setPage] = useState<EventPage | null>(null);
  const [selected, setSelected] = useState<EventItem | null>(null);

  useEffect(() => {
    let cancelled = false;
    searchEvents({ range, query, limit: 200, offset: 0 }).then((p) => {
      if (!cancelled) setPage(p);
    });
    return () => {
      cancelled = true;
    };
  }, [range, query]);

  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = useCallback((next: Term[]) => setQuery(formatQuery(next)), []);
  const addFilter = useCallback((k: string, v: string) => setQuery((q) => addTerm(q, k, v)), []);
  const facets = useMemo(() => (page?.facets ?? []).map((f) => ({ ...f, values: f.values })), [page]);

  return (
    <div className="ev-explorer">
      <div className="section-header">
        <div className="section-header-left">
          <div className="page-tab">
            <PanelLeft size={16} /> Views
          </div>
          <div className="page-tab active">
            My View
            <EllipsisVertical size={16} className="muted" />
          </div>
        </div>
        <TimePicker />
      </div>

      <div className="ev-scroll">
        <div className="ev-query">
          <QueryBar value={query} onChange={setQuery} placeholder="Filter your events" />
          <button className="btn ev-code" title="Query syntax">
            <Code2 size={15} />
          </button>
        </div>

        <div className="viz-row">
          <div className="btn-group">
            <span className="btn btn-label">Visualize as</span>
            {VIZ.map(({ key, label, icon: Icon }) => (
              <button key={key} className={`btn${viz === key ? ' btn-active' : ''}`} onClick={() => setViz(key)}>
                <Icon size={14} /> {label}
              </button>
            ))}
          </div>
        </div>

        <div className="ev-histogram">
          <Histogram page={page} height={viz === 'timeseries' ? 220 : 80} />
        </div>

        <div className="ev-body">
          {showFacets && page && (
            <FacetPanel
              facets={facets}
              terms={terms}
              onChange={setTerms}
              mode="exclude"
              valueSearch
              defaultOpen={['source', 'status']}
              verified={['source', 'status', 'host']}
              renderValueIcon={(key, value) => (key === 'source' ? <SourceIcon source={value} size={14} /> : null)}
              header={
                <div className="ev-teams">
                  <ChevronDown size={14} className="rot-90" /> Teams <Pin size={13} className="muted" />
                </div>
              }
            />
          )}

          <div className="ev-results">
            <div className="ev-list-card">
              <div className="ev-toolbar">
                <button className="ev-toggle" onClick={() => setShowFacets((s) => !s)} title={showFacets ? 'Hide facets' : 'Show facets'}>
                  {showFacets ? <PanelLeftClose size={16} /> : <PanelLeft size={16} />}
                </button>
                <span className="ev-count">{compact(page?.total ?? 0).replace('K', 'k')} Events</span>
                <div className="toolbar-right">
                  <div className="btn-group">
                    <button className="btn">
                      <LayoutDashboard size={14} /> Save to Dashboard
                    </button>
                    <button className="btn">
                      <ChevronDown size={14} />
                    </button>
                  </div>
                  <div className="btn-group">
                    <button className="btn" onClick={() => navigator.clipboard?.writeText(query)}>
                      <Copy size={14} /> Copy
                    </button>
                    <button className="btn">
                      <ChevronDown size={14} />
                    </button>
                  </div>
                  <button className="toolbar-link">
                    <Settings size={15} /> Options
                  </button>
                </div>
              </div>

              {viz === 'list' || viz === 'timeseries' ? (
                <table className="ev-table">
                  <thead>
                    <tr>
                      <th className="ev-col-source">SOURCE</th>
                      <th>TITLE</th>
                      <th className="ev-col-date">DATE</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(page?.items ?? []).map((e) => (
                      <tr key={e.id} className={selected?.id === e.id ? 'selected' : ''} onClick={() => setSelected(e)}>
                        <td className="ev-col-source">
                          <SourceIcon source={e.source} size={26} />
                        </td>
                        <td>
                          <div className="ev-title-line">
                            <StatusBadge status={e.alertType} />
                            <span className="ev-title">{e.title}</span>
                            {e.source && <span className="ev-via">via {e.source}</span>}
                          </div>
                          {e.text && <div className="ev-text">{eventSnippet(e.text)}</div>}
                          {e.tags.length > 0 && (
                            <div className="ev-tags">
                              {e.tags.slice(0, TAGS_SHOWN).map((t) => (
                                <span key={t} className="ev-tag">
                                  {t}
                                </span>
                              ))}
                              {e.tags.length > TAGS_SHOWN && <span className="ev-tag ev-tag-more">+{e.tags.length - TAGS_SHOWN}</span>}
                            </div>
                          )}
                        </td>
                        <td className="ev-col-date" title={formatLongDate(e.timestamp)}>
                          {timeAgo(e.timestamp)}
                        </td>
                      </tr>
                    ))}
                    {page && page.items.length === 0 && (
                      <tr>
                        <td colSpan={3} className="ev-empty">
                          No events match this query in the selected time range.
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              ) : (
                page && <Aggregate viz={viz} page={page} />
              )}
            </div>
          </div>
        </div>
      </div>

      {selected && <EventDetail event={selected} onClose={() => setSelected(null)} onFilter={addFilter} />}
    </div>
  );
}
