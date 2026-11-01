import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { ArrowRight, ChevronRight, ExternalLink, FileText, Plus, Tag } from 'lucide-react';
import { listBudgets, type CostQueryResult } from '../../api/cloudCost/client';
import { searchMonitors, type MonitorItem } from '../../api/monitors/client';
import { Chart } from '../../components/charts/Chart';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { timeAgo } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { StatusBadge } from '../monitors/common';
import {
  DAY,
  daysInMonth,
  filterQuery,
  money,
  moneyAxis,
  monthName,
  monthStart,
  pctChange,
  pctLabel,
  providerColor,
  providerVendor,
  shortDate,
  signedMoney,
  today,
  useCosts,
  useCostTags,
  type Filters,
} from './costModel';
import { CloudCostIntro } from './CloudCostIntro';
import { ChangePill, CostFilterBar, ProviderIcon, SectionTitle, ShareBar, TrendIcon } from './widgets';

type Trend = '3m' | '6m' | '1y' | 'mtd' | 'ytd';
const TRENDS: { value: Trend; label: string }[] = [
  { value: '3m', label: '3 Months' },
  { value: '6m', label: '6 Months' },
  { value: '1y', label: '1 Year' },
  { value: 'mtd', label: 'Month to Date' },
  { value: 'ytd', label: 'Year to Date' },
];
const FILTER_KEYS = ['providername', 'servicename', 'subaccountname', 'regionname', 'team'];
const FILTER_LABELS: Record<string, string> = { providername: 'Provider', servicename: 'Servicename', subaccountname: 'Sub Account Name', regionname: 'Region Name', team: 'Team' };

function trendStart(t: Trend, now: number): number {
  switch (t) {
    case '3m':
      return monthStart(now, -2);
    case '6m':
      return monthStart(now, -5);
    case '1y':
      return monthStart(now, -11);
    case 'mtd':
      return monthStart(now);
    case 'ytd':
      return Date.UTC(new Date(now).getUTCFullYear(), 0, 1);
  }
}

interface MonthStat {
  start: number;
  total: number;
  days: number;
}

/** Daily costs per month of the range; the current month counts the days billed so far. */
function months(res: CostQueryResult): MonthStat[] {
  const through = res.dataThrough ?? today() - DAY;
  const out = new Map<number, MonthStat>();
  res.buckets.forEach((b, i) => {
    if (b > through) return;
    const m = monthStart(b);
    const s = out.get(m) ?? { start: m, total: 0, days: 0 };
    s.total += res.series.reduce((a, x) => a + x.values[i], 0);
    s.days += 1;
    out.set(m, s);
  });
  return [...out.values()].sort((a, b) => a.start - b.start);
}

export function OverviewPage() {
  const [filters, setFilters] = useState<Filters>({});
  const now = today();
  const tagRange = useMemo(() => ({ from: now - 90 * DAY, to: now + DAY - 1 }), [now]);
  const tags = useCostTags(tagRange);
  const query = filterQuery(filters);
  // Any cost in the past year: without one, the overview is the product intro.
  const yearRange = useMemo(() => ({ from: monthStart(now, -11), to: now + DAY - 1 }), [now]);
  const anyCost = useCosts({ range: yearRange, interval: 'month' });
  if (anyCost.data && !anyCost.data.series.length) return <CloudCostIntro />;
  return (
    <div className="ccm-page">
      <CostFilterBar keys={FILTER_KEYS} labels={FILTER_LABELS} filters={filters} onChange={setFilters} tags={tags.data} />
      <div className="ccm-overview">
        <SpendTrends query={query} />
        <CostChanges query={query} />
        <Budgets />
        <Allocation query={query} />
        <CostMonitors />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- spend trends

function SpendTrends({ query }: { query: string }) {
  const [trend, setTrend] = useState<Trend>('3m');
  const [usageOnly, setUsageOnly] = useState(false);
  const now = today();
  const from = trendStart(trend, now);
  const end = monthStart(now, 1) - DAY;
  const q = useCosts({ query, range: { from, to: now + DAY - 1 }, interval: 'day', groupBy: ['providername'], usageOnly });
  return (
    <section className="ccm-block">
      <SectionTitle
        right={
          <span className="btn-group">
            {TRENDS.map((t) => (
              <button key={t.value} className={`btn${trend === t.value ? ' btn-active' : ''}`} onClick={() => setTrend(t.value)}>
                {t.label}
              </button>
            ))}
          </span>
        }
      >
        Understand spend trends
      </SectionTitle>
      {q.isPending ? (
        <Loading label="Loading costs…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !q.data.series.length ? (
        <Empty>No costs yet. Upload custom costs files through the Cloud Cost API (PUT /api/v2/cost/custom_costs) to see them here.</Empty>
      ) : (
        <TrendBody res={q.data} from={from} end={end} usageOnly={usageOnly} onUsageOnly={setUsageOnly} query={query} />
      )}
    </section>
  );
}

function TrendBody({ res, from, end, usageOnly, onUsageOnly, query }: { res: CostQueryResult; from: number; end: number; usageOnly: boolean; onUsageOnly: (v: boolean) => void; query: string }) {
  const stats = months(res);
  const current = stats.find((s) => s.start === monthStart(today()));
  const previous = stats.filter((s) => s !== current);
  const avg = current && current.days ? current.total / current.days : 0;
  const prevAvg =
    previous.reduce((a, s) => a + s.total, 0) /
    Math.max(
      1,
      previous.reduce((a, s) => a + s.days, 0),
    );
  const diff = avg - prevAvg;
  const forecast = current ? current.total + avg * (daysInMonth(current.start) - current.days) : 0;
  const shown = previous.slice(-2);
  return (
    <>
      {current && previous.length > 0 && (
        <p className="ccm-headline">
          <TrendIcon up={diff > 0} />
          <span>
            <b>{monthName(current.start)} daily average</b> is projected to be{' '}
            <b className={diff > 0 ? 'ccm-up' : 'ccm-down'}>
              {money(Math.abs(diff), { precise: true })} {diff > 0 ? 'higher' : 'lower'} ({pctLabel(pctChange(avg, prevAvg), 2)})
            </b>{' '}
            than previous {previous.length === 1 ? 'month' : `${previous.length} months`}
          </span>
        </p>
      )}
      <div className="ccm-stat-cards">
        {shown.map((s) => (
          <div key={s.start} className="ccm-stat-card">
            <span>{monthName(s.start)} total spend</span>
            <b>{money(s.total)}</b>
          </div>
        ))}
        {current && (
          <div className="ccm-stat-card ccm-stat-card-split">
            <div>
              <span>{monthName(current.start)} MTD spend</span>
              <b>{money(current.total)}</b>
            </div>
            <div>
              <span>Forecast EOM total</span>
              <b>{money(forecast)}</b>
            </div>
          </div>
        )}
      </div>
      <div className="ccm-chart-caption">
        <b>Amortized Daily Spend</b>
        <button className={`link-btn${usageOnly ? ' ccm-on' : ' muted-text'}`} onClick={() => onUsageOnly(!usageOnly)} title="Leave out purchases, taxes, credits and adjustments">
          Usage Charges Only
        </button>
      </div>
      <div className="ccm-trend-chart">
        <Chart option={trendChart(res, from, end, stats)} height="100%" />
      </div>
      <div className="ccm-legend">
        <b>providername:</b>
        {res.series.map((s) => (
          <span key={s.tags.providername}>
            <i style={{ background: providerColor(s.tags.providername) }} /> {s.tags.providername}
          </span>
        ))}
      </div>
      <div className="ccm-block-foot">
        <Link to={`/cloud-cost/explorer?${new URLSearchParams({ q: query, groupBy: 'providername', range: '3mo' })}`}>
          <FileText size={14} /> Report on These Spend Trends <ExternalLink size={13} />
        </Link>
      </div>
    </>
  );
}

function trendChart(res: CostQueryResult, from: number, end: number, stats: MonthStat[]): EChartsOption {
  const days: number[] = [];
  for (let d = from; d <= end; d += DAY) days.push(d);
  const index = new Map(res.buckets.map((b, i) => [b, i]));
  const through = res.dataThrough ?? today() - DAY;
  const current = stats.find((s) => s.start === monthStart(today()));
  const avg = current && current.days ? current.total / current.days : 0;
  const muted = token('--text-muted');
  const border = token('--border');
  const monthFirst = stats.map((s) => days.indexOf(s.start)).filter((i) => i >= 0);
  // Month bands with their names, and each month's daily average.
  const bands = stats.map((s) => {
    const a = Math.max(0, days.indexOf(s.start));
    const last = Math.min(days.length - 1, a + daysInMonth(s.start) - 1);
    return { s, a, last };
  });
  const errorColor = token('--status-error');
  return {
    grid: { left: 8, right: 8, top: 26, bottom: 4, containLabel: true },
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'shadow' },
      valueFormatter: (v) => money(Number(v), { precise: true }),
    },
    xAxis: {
      type: 'category',
      data: days.map(shortDate),
      axisLabel: { interval: Math.max(0, Math.ceil(days.length / 24) - 1), color: muted },
      axisTick: { show: false },
    },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v), color: muted }, splitLine: { lineStyle: { color: border } } },
    series: [
      ...res.series.map((s, si) => ({
        name: s.tags.providername,
        type: 'bar' as const,
        stack: 'cost',
        barCategoryGap: '18%',
        color: providerColor(s.tags.providername),
        data: days.map((d) => (index.has(d) && d <= through ? s.values[index.get(d)!] : null)),
        ...(si === 0 && {
          markArea: {
            silent: true,
            itemStyle: { color: 'transparent' },
            label: { position: 'top' as const, color: muted, fontWeight: 600, fontSize: 11 },
            data: bands.map(({ s: m, a, last }) => [{ name: monthName(m.start).toUpperCase(), xAxis: a }, { xAxis: last }]) as never,
          },
          markLine: {
            silent: true,
            symbol: 'none',
            label: { show: true, position: 'insideStartTop' as const, fontSize: 10, color: muted, backgroundColor: token('--bg'), padding: [2, 4] },
            data: [
              ...monthFirst.map((i) => ({ xAxis: i, lineStyle: { type: 'dashed' as const, color: muted }, label: { show: false } })),
              ...bands.map(({ s: m, a, last }) => {
                const isCurrent = m === current;
                const value = m.days ? m.total / m.days : 0;
                const color = isCurrent ? errorColor : muted;
                return [{ coord: [a, value], lineStyle: { type: 'solid' as const, color, width: 1 }, label: { formatter: `Daily Avg ${moneyAxis(value)}`, color } }, { coord: [last, value] }];
              }),
            ] as never,
          },
        }),
      })),
      // The rest of the month at this month's daily average.
      {
        name: 'Forecast',
        type: 'bar' as const,
        stack: 'cost',
        color: token('--chart-1'),
        itemStyle: { opacity: 0.35, decal: { symbol: 'rect', dashArrayX: [1, 0], dashArrayY: [2, 4], rotation: -Math.PI / 4, color: 'rgba(255,255,255,0.6)' } },
        data: days.map((d) => (d > through && current && d >= current.start ? Math.round(avg * 100) / 100 : null)),
      },
    ],
  };
}

// ---------------------------------------------------------------- cost changes

const PERIODS = [
  { value: '7', label: 'Past 7 Days vs 7 Days Before' },
  { value: '14', label: 'Past 14 Days vs 14 Days Before' },
  { value: '30', label: 'Past 30 Days vs 30 Days Before' },
];

interface Change {
  provider: string;
  service: string;
  cost: number;
  prev: number;
  delta: number;
}

function CostChanges({ query }: { query: string }) {
  const [period, setPeriod] = useState('7');
  const [picked, setPicked] = useState<string | null>(null);
  const days = Number(period);
  // Costs are billed a day late: the period ends yesterday.
  const end = today() - DAY;
  const range = { from: end - (days - 1) * DAY, to: end + DAY - 1 };
  const q = useCosts({ query, range, interval: 'month', groupBy: ['providername', 'servicename'] });
  const changes: Change[] = (q.data?.series ?? []).map((s) => ({ provider: s.tags.providername, service: s.tags.servicename, cost: s.total, prev: s.previousTotal, delta: s.total - s.previousTotal }));
  const top = [...changes].sort((a, b) => Math.abs(b.delta) - Math.abs(a.delta)).slice(0, 5);
  const key = (c: Change) => `${c.provider}/${c.service}`;
  const selected = top.find((c) => key(c) === picked) ?? top[0];
  const total = q.data?.total ?? 0;
  const prev = q.data?.previousTotal ?? 0;
  return (
    <section className="ccm-block">
      <SectionTitle
        right={
          <Select
            value={period}
            options={PERIODS.map((p) => ({ value: p.value, label: p.label, icon: <span className="ccm-period">{p.value}d</span> }))}
            onChange={setPeriod}
            triggerClassName="dd-trigger ccm-period-select"
          />
        }
      >
        Investigate cost changes
      </SectionTitle>
      {q.isPending ? (
        <Loading label="Loading cost changes…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !changes.length ? (
        <Empty>No costs in the past {days} days.</Empty>
      ) : (
        <>
          <p className="ccm-headline">
            <TrendIcon up={total > prev} />
            <span>
              <b>Past {days} days spend</b> is{' '}
              <b className={total > prev ? 'ccm-up' : 'ccm-down'}>
                {money(Math.abs(total - prev))} {total > prev ? 'higher' : 'lower'} ({pctLabel(pctChange(total, prev))})
              </b>{' '}
              than previous {days} days
            </span>
          </p>
          <div className="ccm-changes">
            <div className="ccm-changes-chart">
              <div className="ccm-card-head">
                <span>Top 5 Cost Changes by Service Name</span>
                <span className="muted-text">Amortized</span>
              </div>
              <div className="ccm-waterfall">
                <Chart
                  option={waterfall(top, changes, total, prev, range.from, days, selected && key(selected))}
                  height="100%"
                  onClick={(p) => p.dataIndex > 0 && p.dataIndex <= top.length && setPicked(key(top[p.dataIndex - 1]))}
                />
              </div>
            </div>
            {selected && <ChangeDetail change={selected} query={query} range={range} days={days} />}
          </div>
        </>
      )}
    </section>
  );
}

function waterfall(top: Change[], all: Change[], total: number, prev: number, from: number, days: number, selected: string | undefined): EChartsOption {
  const other = all.filter((c) => !top.includes(c)).reduce((a, c) => a + c.delta, 0);
  const steps = [...top.map((c) => ({ label: c.service, delta: c.delta, key: `${c.provider}/${c.service}` })), { label: 'Other', delta: other, key: 'other' }];
  const labels = [
    `${days} Days Before\n(${shortDate(from - days * DAY)} – ${shortDate(from - DAY)})`,
    ...steps.map((s) => s.label),
    `Past ${days} Days\n(${shortDate(from)} – ${shortDate(from + (days - 1) * DAY)})`,
  ];
  const base: (number | null)[] = [0];
  const bars: { value: number; itemStyle: { color: string } }[] = [{ value: prev, itemStyle: { color: token('--bg-muted') } }];
  let running = prev;
  const up = token('--status-error');
  const down = token('--status-ok');
  for (const s of steps) {
    const next = running + s.delta;
    base.push(Math.min(running, next));
    const color = s.delta > 0 ? up : down;
    bars.push({ value: Math.abs(s.delta), itemStyle: { color: s.key === selected ? color : `${color}66` } });
    running = next;
  }
  base.push(0);
  bars.push({ value: total, itemStyle: { color: token('--bg-muted') } });
  const muted = token('--text-muted');
  return {
    grid: { left: 8, right: 8, top: 24, bottom: 4, containLabel: true },
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'shadow' },
      formatter: (ps) => {
        const p = (ps as { dataIndex: number }[])[0];
        const i = p.dataIndex;
        const v = i === 0 ? prev : i === labels.length - 1 ? total : steps[i - 1].delta;
        return `${labels[i].replace('\n', ' ')}<br/><b>${i === 0 || i === labels.length - 1 ? money(v, { precise: true }) : signedMoney(v)}</b>`;
      },
    },
    xAxis: {
      type: 'category',
      data: labels,
      axisLabel: { interval: 0, color: muted, fontSize: 11, lineHeight: 13, formatter: (v: string) => (v.includes('\n') || v.length <= 11 ? v : `${v.slice(0, 10)}…`) },
      axisTick: { show: false },
    },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v), color: muted } },
    series: [
      { type: 'bar', stack: 'w', silent: true, itemStyle: { color: 'transparent' }, data: base, barWidth: '80%', barMaxWidth: 90 },
      {
        type: 'bar',
        stack: 'w',
        barWidth: '80%',
        barMaxWidth: 90,
        data: bars,
        label: {
          show: true,
          position: 'top',
          fontSize: 11,
          color: token('--text'),
          formatter: (p) => {
            const i = (p as { dataIndex: number }).dataIndex;
            if (i === 0) return moneyAxis(prev);
            if (i === labels.length - 1) return moneyAxis(total);
            const d = steps[i - 1].delta;
            return Math.abs(d) >= 1000 ? `${d < 0 ? '-' : ''}${moneyAxis(Math.abs(d))}` : `${d < 0 ? '-' : ''}$${Math.abs(d).toFixed(2)}`;
          },
        },
      },
    ],
  };
}

function ChangeDetail({ change, query, range, days }: { change: Change; query: string; range: { from: number; to: number }; days: number }) {
  const scope = filterQuery({ providername: [change.provider], servicename: [change.service] }, query);
  const q = useCosts({ query: scope, range, interval: 'month', groupBy: ['subaccountname'] });
  const pct = pctChange(change.cost, change.prev);
  const rows = (q.data?.series ?? [])
    .map((s) => ({ name: s.tags.subaccountname, cost: s.total, delta: s.total - s.previousTotal, pct: pctChange(s.total, s.previousTotal) }))
    .sort((a, b) => Math.abs(b.delta) - Math.abs(a.delta));
  const max = Math.max(1, ...rows.map((r) => Math.abs(r.delta)));
  const up = change.delta > 0;
  return (
    <div className={`ccm-change-detail ${up ? 'ccm-change-up' : 'ccm-change-down'}`}>
      <div className="ccm-change-title">
        <ProviderIcon provider={change.provider} service={change.service} size={44} />
        <div>
          <h3>
            <span className="muted-text">{providerVendor(change.provider)}</span> {change.service}
          </h3>
          <p>
            spend past {days} days is <b>{money(change.cost, { precise: true })}</b>, which {up ? 'increased' : 'decreased'} by{' '}
            <b className={up ? 'ccm-up' : 'ccm-down'}>
              {money(Math.abs(change.delta), { precise: true })} ({pctLabel(pct).replace('+', '')})
            </b>{' '}
            from the previous {days} days
          </p>
        </div>
      </div>
      <h4>Top Cost Changes by Sub Account Name</h4>
      {q.isPending ? (
        <Loading />
      ) : (
        rows.slice(0, 6).map((r) => (
          <Link
            key={r.name}
            className="ccm-change-row"
            to={`/cloud-cost/explorer?${new URLSearchParams({ q: filterQuery({ subaccountname: [r.name] }, scope), groupBy: 'subaccountname', range: '1mo' })}`}
          >
            <ProviderIcon provider={change.provider} />
            <span className="ccm-change-name">
              {r.name}
              <small>{money(r.cost, { precise: true })}</small>
            </span>
            <b className={r.delta > 0 ? 'ccm-up' : 'ccm-down'}>{signedMoney(r.delta).replace('+', '')}</b>
            <ShareBar share={Math.abs(r.delta) / max} tone={r.delta > 0 ? 'red' : 'green'} />
            <ChangePill pct={r.pct} digits={1} />
            <ChevronRight size={16} className="muted-text" />
          </Link>
        ))
      )}
    </div>
  );
}

// ---------------------------------------------------------------- budgets, allocation, monitors

function Budgets() {
  const q = useQuery({ queryKey: ['cloud-cost', 'budgets'], queryFn: ({ signal }) => listBudgets(signal) });
  const active = (q.data ?? []).filter((b) => b.month != null);
  return (
    <section className="ccm-block">
      <SectionTitle>Plan with budgets</SectionTitle>
      {q.isPending ? (
        <Loading />
      ) : !active.length ? (
        <p className="ccm-headline">
          <TrendIcon up={null} />
          <span>
            <b>No budgets{q.data?.length ? ' for this month' : ''}.</b> <Link to="/cloud-cost/budgets">Create a new budget</Link> to help teams track spend and prevent overruns.
          </span>
        </p>
      ) : (
        <div className="ccm-budget-cards">
          {active.map((b) => {
            const over = b.monthForecast > b.monthAmount;
            return (
              <Link key={b.id} to="/cloud-cost/budgets" className="ccm-budget-card">
                <b>{b.name}</b>
                <span className="muted-text">{b.metricsQuery}</span>
                <ShareBar share={b.monthAmount ? b.monthActual / b.monthAmount : 0} tone={over ? 'red' : 'blue'} />
                <span>
                  {money(b.monthActual)} of {money(b.monthAmount)} · forecast <b className={over ? 'ccm-up' : ''}>{money(b.monthForecast)}</b>
                </span>
              </Link>
            );
          })}
        </div>
      )}
    </section>
  );
}

const ALLOCATION_TAGS = ['team', 'service', 'env'];

function Allocation({ query }: { query: string }) {
  const now = today();
  const range = useMemo(() => ({ from: now - 30 * DAY, to: now + DAY - 1 }), [now]);
  const tags = useCostTags(range, query);
  return (
    <section className="ccm-block">
      <SectionTitle
        right={
          <Link to="/cloud-cost/tags">
            All tags <ArrowRight size={14} />
          </Link>
        }
      >
        Allocate costs
      </SectionTitle>
      <div className="ccm-allocation">
        {ALLOCATION_TAGS.map((k) => {
          const t = tags.data?.find((x) => x.key === k);
          const coverage = t?.coverage ?? 0;
          return (
            <div key={k} className="ccm-allocation-card">
              <span className="ccm-allocation-icon">
                <Tag size={18} />
              </span>
              <div>
                <b>{k}</b>
                <span className="muted-text">{t ? `${t.distinctValues} values` : tags.isPending ? '…' : 'not on any cost'}</span>
                <div className="ccm-coverage">
                  <ShareBar share={coverage} />
                  <span>{(coverage * 100).toFixed(1)}%</span>
                </div>
                <small className="muted-text">of the last 30 days' cost is tagged</small>
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}

const COST_METRIC = /\b[a-z]+\.cost(\.[a-z.]+)?\{/;

function CostMonitors() {
  const q = useQuery({
    queryKey: ['cloud-cost', 'monitors'],
    queryFn: () =>
      searchMonitors({
        query: 'cost',
        statuses: ['alert', 'warn'],
        types: ['metric'],
        creators: [],
        drafts: [],
        muted: [],
        priorities: [],
        recipients: [],
        teams: [],
        sort: 'status',
        offset: 0,
        limit: 50,
      }),
  });
  const monitors: MonitorItem[] = (q.data?.monitors ?? []).filter((m) => COST_METRIC.test(m.query));
  return (
    <section className="ccm-block">
      <SectionTitle
        right={
          <span className="ccm-actions">
            <Link className="btn" to="/monitors?q=cost">
              View All Cost Monitors <ArrowRight size={14} />
            </Link>
            <Link className="btn btn-primary" to={`/monitors/new?type=metric&query=${encodeURIComponent('sum:all.cost{*}')}`}>
              <Plus size={14} /> New Cost Monitor
            </Link>
          </span>
        }
      >
        Resolve recently triggered monitors
      </SectionTitle>
      {q.isPending ? (
        <Loading />
      ) : !monitors.length ? (
        <Empty>No cost monitor is alerting. Cost monitors are metric monitors on the cost metrics (all.cost, aws.cost.amortized, …).</Empty>
      ) : (
        <table className="ccm-table">
          <thead>
            <tr>
              <th>STATUS / TRIGGERED</th>
              <th>PRIORITY</th>
              <th>NAME AND TAGS</th>
              <th>QUERY</th>
            </tr>
          </thead>
          <tbody>
            {monitors.map((m) => (
              <tr key={m.id}>
                <td>
                  <StatusBadge status={m.status} />
                  <div className="muted-text">{timeAgo(m.stateChangedAt)}</div>
                </td>
                <td>{m.priority ? `P${m.priority}` : '—'}</td>
                <td>
                  <Link to={`/monitors/${m.id}`}>{m.name}</Link>
                  <div className="ccm-chips">
                    {m.tags.map((t) => (
                      <span key={t} className="ccm-chip">
                        {t}
                      </span>
                    ))}
                  </div>
                </td>
                <td className="ccm-mono">{m.query}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
