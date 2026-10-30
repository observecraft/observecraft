import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowRight } from 'lucide-react';
import type { CostInterval, CostQueryResult } from '../../api/cloudCost/client';
import { Chart } from '../../components/charts/Chart';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { token } from '../../theme/chartTheme';
import { colorOf } from './CostBoardPage';
import { DAY, RANGES, filterQuery, groupLabel, money, moneyAxis, pctChange, rangeLabel, rangeOf, shortDate, signedMoney, today, useCosts, useCostTags, type Filters } from './costModel';
import { ChangePill, CostFilterBar, ProviderIcon, SectionTitle, ShareBar } from './widgets';

/** AI costs: the costs the bills tag with their AI vendor. */
const SCOPE = 'ai_vendor:*';
const GROUPS = [
  { value: 'ai_vendor', label: 'Provider' },
  { value: 'ai_model', label: 'Model' },
  { value: 'team', label: 'Team' },
  { value: 'service', label: 'Service' },
];
const INTERVALS: { value: CostInterval; label: string }[] = [
  { value: 'day', label: 'Daily' },
  { value: 'week', label: 'Weekly' },
  { value: 'month', label: 'Monthly' },
];

function bars(res: CostQueryResult, interval: CostInterval): EChartsOption {
  const muted = token('--text-muted');
  const through = res.dataThrough ?? today() - DAY;
  const processing = res.buckets.findIndex((b) => b > through);
  const fmt = (t: number) => (interval === 'month' ? new Date(t).toLocaleDateString('en-US', { month: 'short', year: 'numeric', timeZone: 'UTC' }) : shortDate(t));
  return {
    grid: { left: 8, right: 8, top: 24, bottom: 4, containLabel: true },
    tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' }, confine: true, valueFormatter: (v) => money(Number(v), { precise: true }) },
    xAxis: { type: 'category', data: res.buckets.map(fmt), axisLabel: { color: muted }, axisTick: { show: false } },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v), color: muted } },
    series: res.series.map((s, i) => ({
      name: groupLabel(s.tags),
      type: 'bar' as const,
      stack: 'cost',
      barCategoryGap: '12%',
      barMaxWidth: 60,
      color: colorOf(groupLabel(s.tags), i),
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
    })),
  };
}

/** Summarize → AI Costs: what the AI providers cost, by provider, model, team or service. */
export function AiCostsPage() {
  const [filters, setFilters] = useState<Filters>({});
  const [groupBy, setGroupBy] = useState('ai_vendor');
  const [rangeKey, setRangeKey] = useState('1mo');
  const [interval, setStep] = useState<CostInterval>('day');
  const range = useMemo(() => rangeOf(rangeKey), [rangeKey]);
  const tags = useCostTags(range, SCOPE);
  const query = filterQuery(filters, SCOPE);
  const q = useCosts({ query, range, interval, groupBy: [groupBy] });
  const byModel = useCosts({ query, range, interval: 'month', groupBy: ['ai_vendor', 'ai_model'] });
  const groupLabelText = GROUPS.find((g) => g.value === groupBy)?.label ?? groupBy;
  const res = q.data;
  const change = res ? res.total - res.previousTotal : 0;
  const isToday = range.to >= today();
  const models = byModel.data?.series ?? [];
  const max = Math.max(1, ...models.map((m) => m.total));
  return (
    <div className="ccm-page">
      <CostFilterBar keys={['ai_vendor', 'ai_model']} labels={{ ai_vendor: 'Provider', ai_model: 'Model' }} filters={filters} onChange={setFilters} tags={tags.data}>
        <span className="ccm-filterbar-right">
          <Select label="Group by" value={groupBy} options={GROUPS} onChange={setGroupBy} triggerClassName="dd-trigger ccm-filter" />
          <Select
            label="UTC"
            value={rangeKey}
            options={RANGES.map((r) => ({ value: r.value, label: r.label, icon: <span className="ccm-period">{r.value}</span> }))}
            onChange={setRangeKey}
            align="right"
            triggerClassName="dd-trigger ccm-period-select"
          />
        </span>
      </CostFilterBar>
      <div className="ccm-overview">
        <h2 className="ccm-page-title">Your Organization's AI Cost Overview</h2>
        {q.isPending ? (
          <Loading label="Loading AI costs…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : !res!.series.length ? (
          <Empty>No AI costs in this range. Costs tagged with an ai_vendor (model inference on Bedrock, Azure AI Foundry, Vertex AI, or an AI provider's custom costs) show here.</Empty>
        ) : (
          <>
            <div className="ccm-ai-cards">
              <div className="ccm-ai-card">
                <span className="ccm-ai-card-title">Total AI cost</span>
                <span className="muted-text">
                  {rangeLabel(range.from, range.to)}
                  {isToday && ' (TODAY)'}
                </span>
                <b>{money(res!.total)}</b>
              </div>
              <div className="ccm-ai-card">
                <span className="ccm-ai-card-title">AI cost change</span>
                <span className="muted-text">VS. {rangeLabel(res!.previousRange.from, res!.previousRange.to)}</span>
                <b className={change > 0 ? 'ccm-up' : 'ccm-down'}>
                  {money(Math.abs(change))} <ChangePill pct={pctChange(res!.total, res!.previousTotal)} />
                </b>
              </div>
            </div>
            <SectionTitle
              right={
                <span className="btn-group">
                  {INTERVALS.map((i) => (
                    <button key={i.value} className={`btn btn-sm${interval === i.value ? ' btn-active' : ''}`} onClick={() => setStep(i.value)}>
                      {i.label}
                    </button>
                  ))}
                </span>
              }
            >
              <span className="ccm-plain-title">
                {INTERVALS.find((i) => i.value === interval)?.label} AI Cost by {groupLabelText}
              </span>
            </SectionTitle>
            <div className="ccm-trend-chart">
              <Chart height="100%" option={bars(res!, interval)} />
            </div>
            <div className="ccm-legend">
              <b>{groupBy}:</b>
              {res!.series.map((s, i) => (
                <span key={groupLabel(s.tags)}>
                  <i style={{ background: colorOf(groupLabel(s.tags), i) }} /> {groupLabel(s.tags)}
                </span>
              ))}
            </div>
            <div className="ccm-block-foot">
              <Link to={`/cloud-cost/explorer?${new URLSearchParams({ q: query, groupBy, range: rangeKey })}`}>
                View in Cost Explorer <ArrowRight size={14} />
              </Link>
            </div>
            <SectionTitle>Models</SectionTitle>
            <table className="ccm-table">
              <thead>
                <tr>
                  <th>PROVIDER</th>
                  <th>MODEL</th>
                  <th className="num">TOTAL COST</th>
                  <th className="num">$ CHANGE</th>
                  <th className="num">% CHANGE</th>
                </tr>
              </thead>
              <tbody>
                {models.map((m) => (
                  <tr key={groupLabel(m.tags)}>
                    <td>
                      <span className="ccm-vendor">
                        <ProviderIcon provider={m.tags.ai_vendor} size={16} /> {m.tags.ai_vendor}
                      </span>
                    </td>
                    <td>{m.tags.ai_model}</td>
                    <td className="num ccm-cost-cell">
                      {money(m.total)} <ShareBar share={m.total / max} />
                    </td>
                    <td className={`num ${m.total > m.previousTotal ? 'ccm-up' : 'ccm-down'}`}>{signedMoney(m.total - m.previousTotal)}</td>
                    <td className="num">
                      <ChangePill pct={pctChange(m.total, m.previousTotal)} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        )}
      </div>
    </div>
  );
}
