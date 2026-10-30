import { useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowRight, Search, X } from 'lucide-react';
import type { CostInterval, CostQueryResult, CostType } from '../../api/cloudCost/client';
import { Chart } from '../../components/charts/Chart';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { token } from '../../theme/chartTheme';
import { colorOf } from './CostBoardPage';
import { RANGES, filterQuery, groupLabel, money, moneyAxis, pctChange, providerLabel, rangeLabel, rangeOf, shortDate, signedMoney, useCosts, useCostTags, type Filters } from './costModel';
import { ChangePill, CostFilterBar, ProviderIcon, ShareBar } from './widgets';

const SHOWN = 8;
const ROLLUPS: { value: CostInterval; label: string }[] = [
  { value: 'day', label: 'daily' },
  { value: 'week', label: 'weekly' },
  { value: 'month', label: 'monthly' },
];
const FILTER_KEYS = ['servicename', 'service', 'team'];

function chart(res: CostQueryResult, interval: CostInterval): EChartsOption {
  const top = res.series.slice(0, SHOWN);
  const rest = res.series.slice(SHOWN);
  const series = rest.length ? [...top, { tags: { other: 'Other' }, values: res.buckets.map((_, i) => rest.reduce((a, s) => a + s.values[i], 0)) }] : top;
  const muted = token('--text-muted');
  const fmt = (t: number) => (interval === 'month' ? new Date(t).toLocaleDateString('en-US', { month: 'short', year: 'numeric', timeZone: 'UTC' }) : shortDate(t));
  const through = res.dataThrough ?? Infinity;
  const processing = res.buckets.findIndex((b) => b > through);
  return {
    grid: { left: 8, right: 8, top: 24, bottom: 4, containLabel: true },
    tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' }, confine: true, valueFormatter: (v) => money(Number(v), { precise: true }) },
    xAxis: { type: 'category', data: res.buckets.map(fmt), axisLabel: { color: muted }, axisTick: { show: false } },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v), color: muted } },
    series: series.map((s, i) => {
      const name = groupLabel(s.tags) || 'All Cost';
      return {
        name,
        type: 'bar' as const,
        stack: 'cost',
        barCategoryGap: '15%',
        barMaxWidth: 60,
        color: s.tags.other ? token('--text-faint') : series.length > 1 ? colorOf(name, i) : token('--chart-1'),
        data: s.values,
        ...(i === 0 &&
          interval === 'day' &&
          processing > 0 && {
            markLine: {
              silent: true,
              symbol: 'none',
              lineStyle: { type: 'dashed' as const, color: muted },
              label: { formatter: 'Data processing', position: 'insideEndTop' as const, color: muted, fontSize: 11 },
              data: [{ xAxis: processing - 0.5 }],
            },
          }),
      };
    }),
  };
}

/** Analyze → Explorer: costs over time, grouped by any tags, with the change from the period before. */
export function CostExplorerPage() {
  const [params, setParams] = useSearchParams();
  const set = (patch: Record<string, string | null>) => {
    const next = new URLSearchParams(params);
    for (const [k, v] of Object.entries(patch)) {
      if (v == null || v === '') next.delete(k);
      else next.set(k, v);
    }
    setParams(next, { replace: true });
  };
  const rangeKey = params.get('range') ?? '1mo';
  const range = useMemo(() => rangeOf(rangeKey), [rangeKey]);
  const search = params.get('q') ?? '';
  const groupBy = (params.get('groupBy') ?? 'providername,servicename').split(',').filter(Boolean);
  const rollup = (params.get('rollup') as CostInterval | null) ?? 'day';
  const costType = (params.get('costType') as CostType | null) ?? 'amortized';
  const usageOnly = params.get('usage') === '1';
  const provider = params.get('provider') ?? '';
  const [filters, setFilters] = useState<Filters>({});
  const [draft, setDraft] = useState(search);
  const [view, setView] = useState<'summary' | 'dod'>('summary');
  const [legendAll, setLegendAll] = useState(false);
  const tags = useCostTags(range);
  const query = filterQuery({ ...filters, ...(provider ? { providername: [provider] } : {}) }, search);
  const q = useCosts({ query, range, interval: rollup, groupBy, costType, usageOnly });
  const res = q.data;
  const keys = (tags.data ?? []).map((t) => t.key).filter((k) => !groupBy.includes(k));
  const change = res ? res.total - res.previousTotal : 0;
  const maxTotal = Math.max(1, ...(res?.series ?? []).map((s) => Math.abs(s.total)));
  const maxDelta = Math.max(1, ...(res?.series ?? []).map((s) => Math.abs(s.total - s.previousTotal)));
  const seeMore = (tagsOf: Record<string, string>) => {
    const extra = Object.entries(tagsOf)
      .filter(([k, v]) => k !== 'other' && v !== 'N/A')
      .map(([k, v]) => `${k}:${/\s/.test(v) ? `"${v}"` : v}`);
    const next = [search, ...extra].filter(Boolean).join(' ');
    setDraft(next);
    set({ q: next });
  };
  const legend = res ? (legendAll ? res.series : res.series.slice(0, SHOWN)) : [];
  const dodBuckets = res ? res.buckets.slice(-7) : [];
  return (
    <div className="ccm-page">
      <div className="ccm-explorer-scope">
        <span className="ccm-filterbar-label">Scope to</span>
        <Select
          label="Provider"
          value={provider}
          options={[{ value: '', label: 'All' }, ...(tags.data?.find((t) => t.key === 'providername')?.values ?? []).map((v) => ({ value: v.value, label: providerLabel(v.value) }))]}
          onChange={(v) => set({ provider: v })}
          triggerClassName="dd-trigger ccm-filter"
        />
        <Select<CostType>
          label="Cost Type"
          value={costType}
          options={[
            { value: 'amortized', label: 'Amortized' },
            { value: 'billed', label: 'Billed' },
          ]}
          onChange={(v) => set({ costType: v === 'amortized' ? null : v })}
          triggerClassName="dd-trigger ccm-filter"
        />
        <span className="muted-text">with</span>
        <Select
          label="Usage Charges Only"
          value={usageOnly ? '1' : ''}
          options={[
            { value: '', label: 'Disabled' },
            { value: '1', label: 'Enabled' },
          ]}
          onChange={(v) => set({ usage: v || null })}
          triggerClassName="dd-trigger ccm-filter"
        />
        <span className="ccm-filterbar-right">
          <Select
            label="UTC"
            value={rangeKey}
            options={RANGES.map((r) => ({ value: r.value, label: r.label, icon: <span className="ccm-period">{r.value}</span> }))}
            onChange={(v) => set({ range: v })}
            align="right"
            triggerClassName="dd-trigger ccm-period-select"
          />
        </span>
      </div>
      <div className="ccm-explorer-query">
        <form
          className="ccm-search"
          onSubmit={(e) => {
            e.preventDefault();
            set({ q: draft });
          }}
        >
          <Search size={15} />
          <input placeholder="Search for tags (servicename:AmazonEC2 team:ml)" value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => draft !== search && set({ q: draft })} />
        </form>
        <div className="ccm-groupby">
          <span className="ccm-addon">Group by</span>
          {groupBy.map((g) => (
            <span key={g} className="ccm-groupby-chip">
              {g}
              <button title="Remove" onClick={() => set({ groupBy: groupBy.filter((x) => x !== g).join(',') || '' })}>
                <X size={12} />
              </button>
            </span>
          ))}
          {groupBy.length < 4 && (
            <Select
              value=""
              options={[{ value: '', label: 'Add…' }, ...keys.map((k) => ({ value: k, label: k }))]}
              onChange={(k) => k && set({ groupBy: [...groupBy, k].join(',') })}
              triggerClassName="dd-trigger ccm-groupby-add"
            />
          )}
        </div>
        <Select label="Rollup" value={rollup} options={ROLLUPS} onChange={(v) => set({ rollup: v === 'day' ? null : v })} triggerClassName="dd-trigger ccm-filter" />
      </div>
      <CostFilterBar prefix="" keys={FILTER_KEYS} labels={{ servicename: 'Servicename', service: 'Service', team: 'Team' }} filters={filters} onChange={setFilters} tags={tags.data} />
      {q.isPending ? (
        <Loading label="Loading costs…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !res!.series.length ? (
        <Empty>No costs match.</Empty>
      ) : (
        <>
          <div className="ccm-explorer-top">
            <div className="ccm-explorer-totals">
              <div>
                <span className="ccm-ai-card-title">Total cost</span>
                <span className="muted-text">{rangeLabel(range.from, Math.min(range.to, res!.dataThrough ?? range.to))}</span>
                <b className="ccm-dotted">{money(res!.total)}</b>
              </div>
              <div>
                <span className="ccm-ai-card-title">Total cost change</span>
                <span className="muted-text">VS. {rangeLabel(res!.previousRange.from, res!.previousRange.to)}</span>
                <b className={change > 0 ? 'ccm-up' : 'ccm-down'}>
                  {money(Math.abs(change))} <ChangePill pct={pctChange(res!.total, res!.previousTotal)} />
                </b>
              </div>
            </div>
            <div className="ccm-explorer-chart">
              <div className="ccm-card-head">
                <span>All Cost</span>
              </div>
              <div className="ccm-explorer-canvas">
                <Chart height="100%" option={chart(res!, rollup)} />
              </div>
              <div className="ccm-legend">
                <b>{groupBy.join(', ') || 'cost'}:</b>
                {legend.map((s, i) => (
                  <span key={groupLabel(s.tags)}>
                    <i style={{ background: colorOf(groupLabel(s.tags), i) }} /> {groupLabel(s.tags)}
                  </span>
                ))}
                {res!.series.length > SHOWN && (
                  <button className="ccm-more" onClick={() => setLegendAll((a) => !a)}>
                    {legendAll ? 'Less' : `+${res!.series.length - SHOWN}`}
                  </button>
                )}
              </div>
            </div>
          </div>
          <div className="ccm-explorer-tabs">
            <span className="btn-group">
              <button className={`btn btn-sm${view === 'summary' ? ' btn-primary' : ''}`} onClick={() => setView('summary')}>
                Summary
              </button>
              <button className={`btn btn-sm${view === 'dod' ? ' btn-primary' : ''}`} onClick={() => setView('dod')}>
                {rollup === 'day' ? 'Day' : rollup === 'week' ? 'Week' : 'Month'} Over {rollup === 'day' ? 'Day' : rollup === 'week' ? 'Week' : 'Month'}
              </button>
            </span>
          </div>
          <table className="ccm-table ccm-explorer-table">
            <thead>
              <tr>
                {groupBy.map((g) => (
                  <th key={g}>{g.toUpperCase()}</th>
                ))}
                {view === 'summary' ? (
                  <>
                    <th className="num">
                      TOTAL COST <span className="muted-text">{rangeLabel(range.from, Math.min(range.to, res!.dataThrough ?? range.to))}</span>
                    </th>
                    <th className="num">
                      $ CHANGE <span className="muted-text">VS. {rangeLabel(res!.previousRange.from, res!.previousRange.to)}</span>
                    </th>
                    <th className="num">% CHANGE</th>
                    <th />
                  </>
                ) : (
                  dodBuckets.map((b) => (
                    <th key={b} className="num">
                      {shortDate(b).toUpperCase()}
                    </th>
                  ))
                )}
              </tr>
            </thead>
            <tbody>
              {res!.series.map((s) => {
                const delta = s.total - s.previousTotal;
                const offset = res!.buckets.length - dodBuckets.length;
                return (
                  <tr key={groupLabel(s.tags)}>
                    {s.tags.other ? (
                      <td colSpan={Math.max(1, groupBy.length)}>Other</td>
                    ) : (
                      groupBy.map((g) => <td key={g}>{g === 'providername' ? <ProviderIcon provider={s.tags[g]} size={20} /> : <span className="ccm-chip">{s.tags[g]}</span>}</td>)
                    )}
                    {view === 'summary' ? (
                      <>
                        <td className="num ccm-cost-cell">
                          {money(s.total)} <ShareBar share={Math.abs(s.total) / maxTotal} />
                        </td>
                        <td className="num ccm-cost-cell">
                          {signedMoney(delta)} <ShareBar share={Math.abs(delta) / maxDelta} tone={delta > 0 ? 'red' : 'green'} />
                        </td>
                        <td className="num">
                          <ChangePill pct={pctChange(s.total, s.previousTotal)} />
                        </td>
                        <td>
                          {!s.tags.other && (
                            <button className="link-btn" onClick={() => seeMore(s.tags)}>
                              See More <ArrowRight size={14} />
                            </button>
                          )}
                        </td>
                      </>
                    ) : (
                      dodBuckets.map((b, i) => {
                        const v = s.values[offset + i];
                        const before = offset + i > 0 ? s.values[offset + i - 1] : null;
                        return (
                          <td key={b} className="num">
                            {money(v)}
                            {before != null && Math.abs(before) > 0.005 && <div className={`ccm-dod ${v > before ? 'ccm-up' : 'ccm-down'}`}>{signedMoney(v - before)}</div>}
                          </td>
                        );
                      })
                    )}
                  </tr>
                );
              })}
            </tbody>
          </table>
          <p className="muted-text ccm-explorer-hint">
            Costs are also cost metrics (all.cost, aws.cost.amortized, azure.cost.amortized, gcp.cost.amortized, custom.cost.basis): use them in dashboards, notebooks and monitors.
          </p>
        </>
      )}
    </div>
  );
}
