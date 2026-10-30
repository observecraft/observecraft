import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowRight, ChevronsLeft, ChevronsRight, Upload } from 'lucide-react';
import { commitments, type CommitmentsResult } from '../../api/cloudCost/client';
import { Chart } from '../../components/charts/Chart';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { palette, token } from '../../theme/chartTheme';
import { DAY, RANGES, money, moneyAxis, rangeLabel, shortDate, tagValues, today, useCostTags } from './costModel';
import { ProviderIcon, ShareBar } from './widgets';

type Range = { from: number; to: number };

interface ProviderDef {
  id: string;
  label: string;
  preview?: boolean;
  /** Commitment types, as the bills name them. */
  kinds: string[];
}

const PROVIDERS: ProviderDef[] = [
  { id: 'aws', label: 'AWS', kinds: ['Savings Plan', 'Reserved Instance'] },
  { id: 'azure', label: 'Azure', preview: true, kinds: ['Savings Plan', 'Reserved Instance', 'Pre-purchase Plan'] },
  { id: 'gcp', label: 'GCP', kinds: ['Committed Use Discount'] },
];

const slug = (kind: string) => kind.toLowerCase().replace(/\s+/g, '-');
const plural = (kind: string) => `${kind}s`;
const pct = (v: number | null | undefined) => (v == null ? '—' : v > 0 && v < 0.01 ? '< 1%' : `${Math.round(v * 100)}%`);

function useRange() {
  const [key, setKey] = useState('3mo');
  const [shift, setShift] = useState(0);
  const r = RANGES.find((x) => x.value === key) ?? RANGES[3];
  const range = useMemo(() => {
    const end = today() - 1 * DAY - shift * r.days * DAY;
    return { from: end - (r.days - 1) * DAY, to: end + DAY - 1 };
  }, [r.days, shift]);
  const control = (
    <span className="ccm-range">
      <Select
        label="UTC"
        value={key}
        options={RANGES.map((x) => ({ value: x.value, label: shift ? rangeLabel(range.from, range.to) : x.label, icon: <span className="ccm-period">{x.value}</span> }))}
        onChange={(v) => (setKey(v), setShift(0))}
        align="right"
        triggerClassName="dd-trigger ccm-period-select"
      />
      <span className="btn-group">
        <button className="btn btn-sm" title="Earlier" onClick={() => setShift((s) => s + 1)}>
          <ChevronsLeft size={14} />
        </button>
        <button className="btn btn-sm" title="Later" disabled={shift === 0} onClick={() => setShift((s) => Math.max(0, s - 1))}>
          <ChevronsRight size={14} />
        </button>
      </span>
    </span>
  );
  return { range, control };
}

function savingsChart(r: CommitmentsResult): EChartsOption {
  const muted = token('--text-muted');
  const green = token('--status-ok');
  return {
    grid: { left: 8, right: 8, top: 16, bottom: 28, containLabel: true },
    tooltip: {
      trigger: 'axis',
      confine: true,
      valueFormatter: (v) => (v == null ? '—' : String(v)),
      formatter: (items) => {
        const list = Array.isArray(items) ? items : [items];
        const head = list[0] ? shortDate(r.buckets[list[0].dataIndex as number]) : '';
        return [head, ...list.map((p) => `${p.marker} ${p.seriesName}: ${p.seriesName === 'Effective Savings Rate' ? pct(p.value as number) : money(Number(p.value), { precise: true })}`)].join(
          '<br/>',
        );
      },
    },
    legend: { bottom: 0, left: 0, textStyle: { color: muted }, icon: 'rect' },
    xAxis: { type: 'category', data: r.buckets.map(shortDate), axisLabel: { color: muted }, axisTick: { show: false } },
    yAxis: [
      { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v), color: muted } },
      { type: 'value', axisLabel: { formatter: (v: number) => `${Math.round(v * 100)}%`, color: muted }, splitLine: { show: false }, min: 0 },
    ],
    series: [
      { name: 'Commitment Costs', type: 'line', data: r.commitmentCost, symbol: 'none', color: green, lineStyle: { width: 2 } },
      { name: 'Equivalent On-Demand Costs', type: 'line', data: r.onDemandEquivalent, symbol: 'none', color: token('--chart-1'), lineStyle: { width: 2 }, areaStyle: { opacity: 0.08 } },
      { name: 'Effective Savings Rate', type: 'line', yAxisIndex: 1, data: r.savingsRate, symbol: 'none', color: token('--text-muted'), lineStyle: { type: 'dashed', width: 1.5 } },
    ],
  };
}

function coverageChart(r: CommitmentsResult, height: 'small' | 'big' = 'small'): EChartsOption {
  const muted = token('--text-muted');
  const colors = palette();
  return {
    grid: { left: 8, right: 8, top: 12, bottom: height === 'small' ? 28 : 32, containLabel: true },
    tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' }, confine: true, valueFormatter: (v) => money(Number(v), { precise: true }) },
    legend: { bottom: 0, left: 0, textStyle: { color: muted }, icon: 'rect', type: 'scroll' },
    xAxis: { type: 'category', data: r.buckets.map(shortDate), axisLabel: { color: muted }, axisTick: { show: false } },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v), color: muted } },
    series: r.coverageByType.map((t, i) => ({
      name: t.kind === 'On-Demand' ? 'On-Demand' : plural(t.kind),
      type: 'bar' as const,
      stack: 'usage',
      barCategoryGap: '8%',
      data: t.values,
      color: t.kind === 'On-Demand' ? token('--chart-1') : colors[(i + 1) % colors.length],
    })),
  };
}

function exportCsv(provider: string, r: CommitmentsResult) {
  const head = ['date', 'commitment_cost', 'on_demand_equivalent', 'effective_savings_rate'];
  const lines = [head.join(','), ...r.buckets.map((b, i) => [new Date(b).toISOString().slice(0, 10), r.commitmentCost[i], r.onDemandEquivalent[i], r.savingsRate[i] ?? ''].join(','))];
  const url = URL.createObjectURL(new Blob([lines.join('\n') + '\n'], { type: 'text/csv' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = `${provider}-commitments.csv`;
  a.click();
  URL.revokeObjectURL(url);
}

function Overview({ provider, r, go }: { provider: ProviderDef; r: CommitmentsResult; go: (view: string) => void }) {
  return (
    <>
      <div className="ccm-cm-section">
        <div className="ccm-cm-head">
          <h2>Commitments Overview</h2>
          <button className="link-btn" onClick={() => exportCsv(provider.id, r)}>
            <Upload size={14} /> Export
          </button>
        </div>
        <div className="ccm-cm-overview">
          <div className="ccm-cm-kpis">
            <div className="ccm-cm-kpi active">
              <span className="ccm-dotted">Effective Savings Rate</span>
              <b>{pct(r.effectiveSavingsRate)}</b>
            </div>
            <div className="ccm-cm-kpi">
              <span className="ccm-dotted">Realized Savings</span>
              <b>{money(r.realizedSavings)}</b>
            </div>
          </div>
          <div className="ccm-widget">
            <div className="ccm-widget-head">
              <span>Commitment Costs vs. Equivalent On-Demand Costs</span>
            </div>
            <div className="ccm-cm-chart">
              <Chart height="100%" option={savingsChart(r)} />
            </div>
          </div>
        </div>
      </div>
      <div className="ccm-cm-split">
        <div>
          <div className="ccm-cm-subhead">
            <h3>Track your coverage and on-demand spend</h3>
            <button className="link-btn" onClick={() => go('coverage')}>
              View more <ArrowRight size={14} />
            </button>
          </div>
          <div className="ccm-cm-coverage">
            <div className={`ccm-cm-cov-card${r.coverage < 0.5 ? ' low' : ''}`}>
              <span>Coverage</span>
              <b>{pct(r.coverage)}</b>
              <hr />
              <span>On-Demand Spend</span>
              <b className="plain">{money(r.onDemandSpend)}</b>
            </div>
            <div className="ccm-cm-cov-chart">
              <Chart height="100%" option={coverageChart(r)} />
            </div>
          </div>
        </div>
        <div>
          <div className="ccm-cm-subhead">
            <h3>Review your commitment utilization</h3>
            <button className="link-btn" onClick={() => go(slug(provider.kinds[0]))}>
              View more <ArrowRight size={14} />
            </button>
          </div>
          <div className="ccm-cm-util">
            {provider.kinds
              .filter((k) => k !== 'Pre-purchase Plan')
              .map((k) => {
                const u = r.utilization.find((x) => x.kind === k);
                const total = u ? u.used + u.unused : 0;
                return (
                  <button key={k} className="ccm-cm-util-card" onClick={() => go(slug(k))}>
                    <span>{k === 'Committed Use Discount' ? 'Commitments Utilization' : `${plural(k)} Utilization`}</span>
                    {total > 0 ? (
                      <>
                        <b>{pct(u!.used / total)}</b>
                        <ShareBar share={u!.used / total} tone={u!.used / total < 0.8 ? 'red' : 'green'} />
                        <small className="muted-text">{money(u!.unused)} unused</small>
                      </>
                    ) : (
                      <small className="muted-text">No active {plural(k)}</small>
                    )}
                  </button>
                );
              })}
          </div>
        </div>
      </div>
    </>
  );
}

function Coverage({ r }: { r: CommitmentsResult }) {
  return (
    <div className="ccm-cm-section">
      <div className="ccm-cm-head">
        <h2>Coverage</h2>
      </div>
      <div className="ccm-cm-coverage big">
        <div className={`ccm-cm-cov-card${r.coverage < 0.5 ? ' low' : ''}`}>
          <span>Coverage</span>
          <b>{pct(r.coverage)}</b>
          <hr />
          <span>On-Demand Spend</span>
          <b className="plain">{money(r.onDemandSpend)}</b>
        </div>
        <div className="ccm-cm-cov-chart">
          <Chart height="100%" option={coverageChart(r, 'big')} />
        </div>
      </div>
      <h3 className="ccm-cm-table-title">Coverage by service and region</h3>
      {r.coverageByService.length ? (
        <table className="ccm-table">
          <thead>
            <tr>
              <th>SERVICE</th>
              <th>REGION</th>
              <th className="num">COVERED (ON-DEMAND VALUE)</th>
              <th className="num">ON-DEMAND SPEND</th>
              <th>COVERAGE</th>
            </tr>
          </thead>
          <tbody>
            {r.coverageByService.map((s) => {
              const share = s.covered + s.onDemand > 0 ? s.covered / (s.covered + s.onDemand) : 0;
              return (
                <tr key={`${s.service}|${s.region}`}>
                  <td>{s.service}</td>
                  <td>{s.region}</td>
                  <td className="num">{money(s.covered)}</td>
                  <td className="num">{money(s.onDemand)}</td>
                  <td>
                    <div className="ccm-coverage">
                      <ShareBar share={share} tone={share < 0.5 ? 'red' : 'green'} />
                      <span>{pct(share)}</span>
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : (
        <Empty>No usage that commitments cover in this range.</Empty>
      )}
    </div>
  );
}

function Inventory({ kind, r }: { kind: string; r: CommitmentsResult }) {
  const list = r.commitments.filter((c) => c.kind === kind);
  return (
    <div className="ccm-cm-section">
      <div className="ccm-cm-head">
        <h2>{plural(kind)}</h2>
      </div>
      {list.length ? (
        <div className="ccm-cm-scroll">
          <table className="ccm-table ccm-cm-inventory">
            <thead>
              <tr>
                <th>ID</th>
                <th>ACCOUNT</th>
                <th>SERVICE</th>
                <th>REGION</th>
                <th>TERM</th>
                <th>PAYMENT</th>
                <th>EXPIRES</th>
                <th className="num">AMORTIZED COST</th>
                <th>UTILIZATION</th>
                <th className="num">UNUSED</th>
                <th className="num">SAVINGS</th>
              </tr>
            </thead>
            <tbody>
              {list.map((c) => (
                <tr key={c.id}>
                  <td className="ccm-mono" title={c.id}>
                    {c.id.split('/').pop()}
                  </td>
                  <td>{c.account}</td>
                  <td>{c.service}</td>
                  <td>{c.region}</td>
                  <td>{c.term || '—'}</td>
                  <td>{c.purchaseOption || '—'}</td>
                  <td>{c.end || '—'}</td>
                  <td className="num">{money(c.amortizedCost)}</td>
                  <td>
                    <div className="ccm-coverage">
                      <ShareBar share={c.utilization} tone={c.utilization < 0.8 ? 'red' : 'green'} />
                      <span>{pct(c.utilization)}</span>
                    </div>
                  </td>
                  <td className="num">{money(c.unusedCost)}</td>
                  <td className={`num${c.savings >= 0 ? ' ccm-down' : ' ccm-up'}`}>{money(c.savings)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <Empty>No active {plural(kind)} in this range.</Empty>
      )}
    </div>
  );
}

/** Plan → Commitment Programs: per provider, what Savings Plans, Reserved Instances and other commitments save, cover and use. */
export function CommitmentsPage() {
  const [params, setParams] = useSearchParams();
  const provider = PROVIDERS.find((p) => p.id === params.get('provider')) ?? PROVIDERS[0];
  const view = params.get('view') ?? 'overview';
  const [account, setAccount] = useState('');
  const { range, control } = useRange();
  const accounts = useCostTags(range as Range, `providername:${provider.id}`);
  const query = account ? `subaccountname:"${account}"` : '';
  const q = useQuery({
    queryKey: ['cloud-cost', 'commitments', provider.id, query, range.from, range.to],
    queryFn: ({ signal }) => commitments({ provider: provider.id, query, range }, signal),
    placeholderData: (prev) => prev,
  });
  const go = (p: string, v: string) => (setParams({ provider: p, view: v }, { replace: true }), p !== provider.id && setAccount(''));
  const kind = provider.kinds.find((k) => slug(k) === view);
  return (
    <div className="ccm-boards">
      <aside className="ccm-boards-nav ccm-cm-nav">
        <h4>Providers</h4>
        {PROVIDERS.map((p) => (
          <div key={p.id}>
            <button className="ccm-board-link" onClick={() => go(p.id, 'overview')}>
              <ProviderIcon provider={p.id} /> {p.label} {p.preview && <span className="ccm-preview-badge">PREVIEW</span>}
            </button>
            {p === provider && (
              <div className="ccm-cm-links">
                {['overview', 'coverage'].map((v) => (
                  <button key={v} className={view === v ? 'active' : ''} onClick={() => go(p.id, v)}>
                    {v === 'overview' ? 'Overview' : 'Coverage'}
                  </button>
                ))}
                <button className={kind ? 'parent' : ''} onClick={() => go(p.id, slug(p.kinds[0]))}>
                  Inventory
                </button>
                <div className="ccm-cm-sublinks">
                  {p.kinds.map((k) => (
                    <button key={k} className={view === slug(k) ? 'active' : ''} onClick={() => go(p.id, slug(k))}>
                      {plural(k)}
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
        ))}
      </aside>
      <div className="ccm-boards-main ccm-cm-main">
        <div className="ccm-cm-bar">
          <span className="ccm-cm-title">
            <ProviderIcon provider={provider.id} size={22} /> {provider.label} Commitments
          </span>
          <Select
            label="Billing Account"
            value={account}
            options={[{ value: '', label: 'All Billing Accounts' }, ...tagValues(accounts.data, 'subaccountname').map((v) => ({ value: v, label: v }))]}
            onChange={setAccount}
            triggerClassName="dd-trigger ccm-filter"
          />
          <span className="ccm-filterbar-right">{control}</span>
        </div>
        {q.isPending ? (
          <Loading label="Loading commitments…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : view === 'coverage' ? (
          <Coverage r={q.data} />
        ) : kind ? (
          <Inventory kind={kind} r={q.data} />
        ) : (
          <Overview provider={provider} r={q.data} go={(v) => go(provider.id, v)} />
        )}
      </div>
    </div>
  );
}
