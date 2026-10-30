import { useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react';
import type { CostQueryResult } from '../../api/cloudCost/client';
import { Chart } from '../../components/charts/Chart';
import { Empty, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { palette, token } from '../../theme/chartTheme';
import { CONTAINER_BOARDS, GPU_BOARDS, NETWORKING_BOARDS, STORAGE_BOARDS, saasBoard, type Board, type Widget } from './boards';
import {
  DAY,
  RANGES,
  filterQuery,
  groupLabel,
  isCloud,
  providerColor,
  money,
  moneyAxis,
  pctChange,
  pctLabel,
  rangeOf,
  shortDate,
  signedMoney,
  today,
  useCosts,
  useCostTags,
  type Filters,
} from './costModel';
import { ChangePill, CostFilterBar, ProviderIcon, ShareBar } from './widgets';

type Range = { from: number; to: number };

const TOP = 8;

/** A series' color: the provider's for providers, else the palette's in rank order. */
export function colorOf(name: string, index: number): string {
  if (isCloud(name)) return providerColor(name);
  const colors = palette();
  return colors[index % colors.length];
}

function useWidgetCosts(board: Board, w: { scope?: string; groupBy?: string; interval?: 'day' | 'week' | 'month' }, filters: Filters, range: Range) {
  const query = filterQuery(filters, [board.scope, w.scope].filter(Boolean).join(' '));
  return useCosts({ query, range, interval: w.interval ?? 'day', groupBy: w.groupBy ? [w.groupBy] : [], limit: 50 });
}

function Frame({ title, badge, children, className = '' }: { title?: string; badge?: string; children: React.ReactNode; className?: string }) {
  return (
    <div className={`ccm-widget ${className}`}>
      {title && (
        <div className="ccm-widget-head">
          <span>{title}</span>
          {badge && <span className="ccm-badge">{badge}</span>}
        </div>
      )}
      <div className="ccm-widget-body">{children}</div>
    </div>
  );
}

function NoData({ big }: { big?: boolean }) {
  return <div className={`ccm-nodata${big ? ' big' : ''}`}>(No data)</div>;
}

function Kpi({ board, w, filters, range }: { board: Board; w: Extract<Widget, { kind: 'kpi' }>; filters: Filters; range: Range }) {
  const end = today();
  const recent = useMemo(() => ({ from: end - 29 * DAY, to: end + DAY - 1 }), [end]);
  const r = w.value === 'range' ? range : recent;
  const q = useWidgetCosts(board, { scope: w.scope }, filters, r);
  const badge = w.value === 'range' ? undefined : '1mo';
  if (q.isPending)
    return (
      <Frame title={w.title} badge={badge}>
        {<Loading />}
      </Frame>
    );
  const res = q.data;
  const empty = !res || !res.series.length;
  const value = !res ? 0 : w.value === 'prior' ? res.previousTotal : res.total;
  const change = res ? pctChange(res.total, res.previousTotal) : null;
  const tone = w.value === 'change' && change != null ? (change > 0 ? ' ccm-kpi-up' : ' ccm-kpi-down') : '';
  const daily = res && w.spark ? res.buckets.flatMap((b, i) => (b <= (res.dataThrough ?? b) ? [res.series.reduce((a, s) => a + s.values[i], 0)] : [])) : [];
  return (
    <Frame title={w.title} badge={badge} className={`ccm-kpi${tone}`}>
      {empty || (w.value === 'change' && change == null) ? (
        <NoData big />
      ) : (
        <>
          {w.spark && daily.length > 1 && (
            <div className="ccm-kpi-spark">
              <Chart
                height="100%"
                option={{
                  grid: { left: 0, right: 0, top: 0, bottom: 0 },
                  xAxis: { type: 'category', show: false, data: daily.map((_, i) => i) },
                  yAxis: { type: 'value', show: false, min: (v: { min: number }) => v.min * 0.9 },
                  series: [{ type: 'line', data: daily, symbol: 'none', lineStyle: { width: 1, color: token('--text-faint') }, areaStyle: { color: token('--bg-muted') } }],
                }}
              />
            </div>
          )}
          <b className="ccm-kpi-value">{w.value === 'change' ? `${pctLabel(change, 2).replace('+', '')}` : money(value)}</b>
        </>
      )}
    </Frame>
  );
}

function stackedBars(res: CostQueryResult, interval: 'day' | 'week' | 'month'): EChartsOption {
  const top = res.series.slice(0, TOP);
  const rest = res.series.slice(TOP);
  const series = rest.length ? [...top, { tags: { other: 'Other' }, values: res.buckets.map((_, i) => rest.reduce((a, s) => a + s.values[i], 0)), total: 0, previousTotal: 0 }] : top;
  const muted = token('--text-muted');
  const fmt = (t: number) => (interval === 'month' ? new Date(t).toLocaleDateString('en-US', { month: 'short', year: '2-digit', timeZone: 'UTC' }) : shortDate(t));
  return {
    grid: { left: 8, right: 8, top: 12, bottom: series.length > 1 ? 30 : 4, containLabel: true },
    tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' }, confine: true, valueFormatter: (v) => money(Number(v), { precise: true }) },
    legend: series.length > 1 ? { bottom: 0, type: 'scroll', icon: 'rect', itemWidth: 10, itemHeight: 10, textStyle: { color: muted } } : undefined,
    xAxis: { type: 'category', data: res.buckets.map(fmt), axisLabel: { color: muted }, axisTick: { show: false } },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v), color: muted } },
    series: series.map((s, i) => {
      const name = groupLabel(s.tags) || 'cost';
      return {
        name,
        type: 'bar' as const,
        stack: 'cost',
        color: s.tags.other ? token('--text-faint') : series.length > 1 ? colorOf(name, i) : token('--chart-1'),
        data: s.values,
        barCategoryGap: '15%',
        barMaxWidth: 60,
      };
    }),
  };
}

function Bars({ board, w, filters, range }: { board: Board; w: Extract<Widget, { kind: 'bars' }>; filters: Filters; range: Range }) {
  const r = useMemo(() => (w.fixedRange ? rangeOf(w.fixedRange) : range), [w.fixedRange, range]);
  const q = useWidgetCosts(board, w, filters, r);
  return (
    <Frame title={w.title} badge={w.fixedRange} className="ccm-chart-widget">
      {q.isPending ? <Loading /> : !q.data?.series.length ? <EmptyChart range={r} /> : <Chart height="100%" option={stackedBars(q.data, w.interval ?? 'day')} />}
    </Frame>
  );
}

/** An empty chart keeps its axes, as an empty dashboard graph does. */
function EmptyChart({ range }: { range: Range }) {
  const days: number[] = [];
  for (let d = range.from; d <= range.to; d += DAY) days.push(d);
  const muted = token('--text-muted');
  return (
    <Chart
      height="100%"
      option={{
        grid: { left: 8, right: 8, top: 12, bottom: 4, containLabel: true },
        xAxis: { type: 'category', data: days.map(shortDate), axisLabel: { color: muted }, axisTick: { show: false } },
        yAxis: { type: 'value', min: 0, max: 150, interval: 50, axisLabel: { color: muted } },
        series: [{ type: 'line', data: [] }],
      }}
    />
  );
}

function Donut({ board, w, filters, range }: { board: Board; w: Extract<Widget, { kind: 'donut' }>; filters: Filters; range: Range }) {
  const q = useWidgetCosts(board, { scope: w.scope, groupBy: w.groupBy, interval: 'month' }, filters, range);
  const series = q.data?.series.filter((s) => s.total > 0) ?? [];
  return (
    <Frame title={w.title} className="ccm-chart-widget">
      {q.isPending ? (
        <Loading />
      ) : !series.length ? (
        <Empty>No Data</Empty>
      ) : (
        <Chart
          height="100%"
          option={{
            tooltip: { trigger: 'item', valueFormatter: (v) => money(Number(v), { precise: true }) },
            title: { text: money(q.data!.total), left: 'center', top: 'center', textStyle: { fontSize: 18, fontWeight: 600, color: token('--text') } },
            series: [
              {
                type: 'pie',
                radius: ['45%', '72%'],
                label: { formatter: (p) => `${p.name}\n${money(Number(p.value))}`, color: token('--text'), fontSize: 11 },
                data: series.slice(0, TOP).map((s, i) => ({ name: groupLabel(s.tags), value: s.total, itemStyle: { color: colorOf(groupLabel(s.tags), i) } })),
              },
            ],
          }}
        />
      )}
    </Frame>
  );
}

function Table({ board, w, filters, range, onPick }: { board: Board; w: Extract<Widget, { kind: 'table' }>; filters: Filters; range: Range; onPick: (key: string, value: string) => void }) {
  const [search, setSearch] = useState('');
  const end = today();
  const recent = useMemo(() => ({ from: end - 29 * DAY, to: end + DAY - 1 }), [end]);
  const q = useWidgetCosts(board, { scope: w.scope, groupBy: w.groupBy, interval: 'month' }, filters, w.changes ? recent : range);
  const rows = (q.data?.series ?? []).filter((s) => !search || groupLabel(s.tags).toLowerCase().includes(search.toLowerCase()));
  const max = Math.max(1, ...rows.map((r) => Math.abs(r.total)));
  return (
    <Frame title={w.title} badge={w.changes ? '1mo' : undefined} className="ccm-table-widget">
      <label className="ccm-search ccm-widget-search">
        <Search size={14} />
        <input placeholder="Search" value={search} onChange={(e) => setSearch(e.target.value)} />
      </label>
      {q.isPending ? (
        <Loading />
      ) : (
        <div className="ccm-widget-table">
          <table className="ccm-table">
            <thead>
              <tr>
                <th>{w.groupBy.toUpperCase()}</th>
                {w.changes ? (
                  <>
                    <th className="num">COST PAST 30 DAYS</th>
                    <th className="num">COST 30 DAYS PRIOR</th>
                    <th className="num">$ CHANGE</th>
                    <th className="num">% CHANGE</th>
                  </>
                ) : (
                  <th className="num">TOTAL COST</th>
                )}
              </tr>
            </thead>
            <tbody>
              {rows.length === 0 && (
                <tr>
                  <td colSpan={5} className="muted-text">
                    N/A
                  </td>
                </tr>
              )}
              {rows.map((r) => {
                const label = groupLabel(r.tags);
                return (
                  <tr key={label} className="ccm-clickable" onClick={() => r.tags[w.groupBy] && r.tags[w.groupBy] !== 'N/A' && onPick(w.groupBy, r.tags[w.groupBy])}>
                    <td>{label}</td>
                    {w.changes ? (
                      <>
                        <td className="num">{money(r.total, { precise: true })}</td>
                        <td className="num">{money(r.previousTotal, { precise: true })}</td>
                        <td className={`num ${r.total > r.previousTotal ? 'ccm-up' : 'ccm-down'}`}>{signedMoney(r.total - r.previousTotal)}</td>
                        <td className="num">
                          <ChangePill pct={pctChange(r.total, r.previousTotal)} digits={1} />
                        </td>
                      </>
                    ) : (
                      <td className="num ccm-cost-cell">
                        {money(r.total, { precise: true })} <ShareBar share={r.total / max} />
                      </td>
                    )}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </Frame>
  );
}

function WidgetView(props: { board: Board; w: Widget; filters: Filters; range: Range; onPick: (key: string, value: string) => void; tone?: string }) {
  const { w } = props;
  const style = { gridColumn: `span ${w.w ?? 12}` };
  return (
    <div style={style} className={`ccm-cell ccm-cell-${w.kind}`}>
      {w.kind === 'kpi' && <Kpi {...props} w={w} />}
      {w.kind === 'bars' && <Bars {...props} w={w} />}
      {w.kind === 'donut' && <Donut {...props} w={w} />}
      {w.kind === 'table' && <Table {...props} w={w} />}
      {w.kind === 'note' && (
        <div className={`ccm-note ccm-note-${props.tone ?? 'green'}`}>
          {w.lines.map((l) => (
            <p key={l}>{l}</p>
          ))}
        </div>
      )}
    </div>
  );
}

function BoardView({ board }: { board: Board }) {
  const [rangeKey, setRangeKey] = useState('1mo');
  const [filters, setFilters] = useState<Filters>({});
  const range = useMemo(() => rangeOf(rangeKey), [rangeKey]);
  const tags = useCostTags(range, board.scope);
  const pick = (key: string, value: string) => setFilters((f) => ({ ...f, [key]: [value] }));
  return (
    <div className="ccm-board">
      <div className="ccm-board-head">
        <h2>
          {board.provider && <ProviderIcon provider={board.provider} size={22} />} {board.title}
        </h2>
      </div>
      <div className="ccm-board-vars">
        <CostFilterBar prefix="" keys={board.vars} filters={filters} onChange={setFilters} tags={tags.data} />
        <Select
          label="UTC"
          value={rangeKey}
          options={RANGES.map((r) => ({ value: r.value, label: r.label, icon: <span className="ccm-period">{r.value}</span> }))}
          onChange={setRangeKey}
          align="right"
          triggerClassName="dd-trigger ccm-period-select"
        />
      </div>
      <div className="ccm-grid ccm-board-header">
        {board.header.map((w, i) => (
          <WidgetView key={i} board={board} w={w} filters={filters} range={range} onPick={pick} />
        ))}
      </div>
      {board.sections.map((s) => (
        <section key={s.title} id={s.title} className="ccm-board-section">
          <div className={`ccm-band ccm-band-${s.tone ?? 'green'}`}>{s.title}</div>
          <div className="ccm-grid">
            {s.widgets.map((w, i) => (
              <WidgetView key={i} board={board} w={w} filters={filters} range={range} onPick={pick} tone={s.tone} />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

const TITLES: Record<string, string> = { containers: 'Container Dashboards', gpu: 'GPU Dashboards', networking: 'Networking Dashboards', storage: 'Storage Dashboards', saas: 'SaaS Dashboards' };

/** Summarize → GPU / Networking / Storage / SaaS Costs: the tab's dashboards, picked in a sidebar. */
export function CostBoardPage({ tab }: { tab: 'containers' | 'gpu' | 'networking' | 'storage' | 'saas' }) {
  const [params, setParams] = useSearchParams();
  const [collapsed, setCollapsed] = useState(false);
  const now = today();
  const providerRange = useMemo(() => ({ from: now - 90 * DAY, to: now + DAY - 1 }), [now]);
  const providers = useCostTags(providerRange);
  const boards = useMemo(() => {
    if (tab === 'containers') return CONTAINER_BOARDS;
    if (tab === 'gpu') return GPU_BOARDS;
    if (tab === 'networking') return NETWORKING_BOARDS;
    if (tab === 'storage') return STORAGE_BOARDS;
    const names =
      providers.data
        ?.find((t) => t.key === 'providername')
        ?.values.map((v) => v.value)
        .filter((p) => !isCloud(p)) ?? [];
    return names.sort((a, b) => a.localeCompare(b)).map(saasBoard);
  }, [tab, providers.data]);
  const board = boards.find((b) => b.id === params.get('board')) ?? boards[0];
  if (tab === 'saas' && providers.isPending) return <Loading label="Loading SaaS providers…" />;
  return (
    <div className={`ccm-boards${collapsed ? ' collapsed' : ''}`}>
      <aside className="ccm-boards-nav">
        <button className="ccm-collapse" onClick={() => setCollapsed((c) => !c)}>
          {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />} {!collapsed && 'Collapse'}
        </button>
        {!collapsed && (
          <>
            <h4>{TITLES[tab]}</h4>
            {boards.map((b) => (
              <div key={b.id}>
                <button className={`ccm-board-link${b === board ? ' active' : ''}`} onClick={() => setParams({ board: b.id }, { replace: true })}>
                  {b.provider ? <ProviderIcon provider={b.provider} /> : <span className="ccm-board-dot" />} {b.label}
                </button>
                {b === board && (
                  <div className="ccm-board-sections">
                    {b.sections.map((s) => (
                      <a key={s.title} href={`#${s.title}`} onClick={(e) => (e.preventDefault(), document.getElementById(s.title)?.scrollIntoView({ behavior: 'smooth' }))}>
                        {s.title}
                      </a>
                    ))}
                  </div>
                )}
              </div>
            ))}
            {!boards.length && <p className="muted-text ccm-boards-empty">No SaaS costs yet: upload them as custom costs files, named by their provider.</p>}
          </>
        )}
      </aside>
      <div className="ccm-boards-main">
        {board ? <BoardView key={board.id} board={board} /> : <Empty>No SaaS costs yet. Upload custom costs files (PUT /api/v2/cost/custom_costs) with the provider's name as ProviderName.</Empty>}
      </div>
    </div>
  );
}
