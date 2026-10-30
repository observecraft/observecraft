import { useMemo } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate, useParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ChevronLeft, Pencil, Trash2 } from 'lucide-react';
import { deleteBudget, getBudget, type BudgetEntry } from '../../api/cloudCost/client';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { token } from '../../theme/chartTheme';
import { monthLabel } from './BudgetsPage';
import { money, moneyAxis } from './costModel';
import { ShareBar } from './widgets';

const label = (e: BudgetEntry) => e.tag_filters.map((f) => f.tag_value).join(' / ') || 'Total';

function chart(months: number[], amount: number[], actual: (number | null)[], forecast: (number | null)[]): EChartsOption {
  const muted = token('--text-muted');
  return {
    grid: { left: 8, right: 8, top: 32, bottom: 4, containLabel: true },
    tooltip: { trigger: 'axis', confine: true, valueFormatter: (v) => (v == null ? '—' : money(Number(v), { precise: true })) },
    legend: { top: 0, left: 0, textStyle: { color: muted } },
    xAxis: { type: 'category', data: months.map(monthLabel), axisLabel: { color: muted }, axisTick: { show: false } },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v), color: muted } },
    series: [
      { name: 'Budget', type: 'bar', data: amount, barMaxWidth: 36, color: token('--chart-1'), itemStyle: { opacity: 0.35 } },
      { name: 'Actual', type: 'bar', data: actual, barMaxWidth: 36, barGap: '-100%', color: token('--chart-1') },
      { name: 'Forecast', type: 'line', data: forecast, symbol: 'circle', symbolSize: 6, color: token('--chart-4'), lineStyle: { type: 'dashed' } },
    ],
  };
}

/** Plan → Budgets → a budget: each month's budget against what it cost and is forecast to cost, per tag combination. */
export function BudgetDetailPage() {
  const { id = '' } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['cloud-cost', 'budget', id], queryFn: ({ signal }) => getBudget(id, signal) });
  const remove = useMutation({
    mutationFn: () => deleteBudget(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['cloud-cost', 'budgets'] });
      navigate('/cloud-cost/budgets');
    },
  });
  const entries = q.data?.attributes.entries ?? [];
  const months = useMemo(() => [...new Set(entries.map((e) => e.month))].sort((a, b) => a - b), [entries]);
  const rows = useMemo(() => {
    const by = new Map<string, BudgetEntry[]>();
    for (const e of entries) by.set(label(e), [...(by.get(label(e)) ?? []), e]);
    return [...by].map(([name, es]) => ({ name, entries: es })).sort((a, b) => a.name.localeCompare(b.name));
  }, [entries]);
  if (q.isPending) return <Loading label="Loading budget…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const a = q.data.attributes;
  const perMonth = (f: (e: BudgetEntry) => number | null | undefined) =>
    months.map((m) => {
      const vals = entries.filter((e) => e.month === m).map(f);
      return vals.every((v) => v == null) ? null : vals.reduce<number>((s, v) => s + (v ?? 0), 0);
    });
  const amount = perMonth((e) => e.amount) as number[];
  const actual = perMonth((e) => e.costs?.actual);
  const forecast = perMonth((e) => e.costs?.forecast);
  const spent = a.costs?.actual ?? 0;
  const projected = a.costs?.forecast ?? 0;
  const total = a.total_amount ?? 0;
  const over = projected > total;
  return (
    <div className="ccm-page">
      <div className="ccm-overview">
        <div className="ccm-budget-head">
          <div>
            <Link to="/cloud-cost/budgets" className="ccm-back">
              <ChevronLeft size={15} /> Budgets
            </Link>
            <h2 className="ccm-page-title">{a.name}</h2>
            <span className="muted-text">
              {monthLabel(a.start_month)} – {monthLabel(a.end_month)} · <span className="ccm-mono">{a.metrics_query}</span>
            </span>
          </div>
          <div className="ccm-actions">
            <Link className="btn" to={`/cloud-cost/budgets/${id}/edit`}>
              <Pencil size={14} /> Edit
            </Link>
            <button className="btn" disabled={remove.isPending} onClick={() => window.confirm(`Delete the budget "${a.name}"? This can't be undone.`) && remove.mutate()}>
              <Trash2 size={14} /> Delete
            </button>
          </div>
        </div>
        <div className="ccm-ai-cards ccm-ai-cards-3">
          <div className="ccm-ai-card">
            <span className="ccm-ai-card-title">Total budget</span>
            <span className="muted-text">{months.length} months</span>
            <b>{money(total)}</b>
          </div>
          <div className="ccm-ai-card">
            <span className="ccm-ai-card-title">Spent so far</span>
            <span className="muted-text">{total ? `${Math.round((spent / total) * 100)}% of the budget` : '—'}</span>
            <b>{money(spent)}</b>
          </div>
          <div className="ccm-ai-card">
            <span className="ccm-ai-card-title">Forecast</span>
            <span className="muted-text">through this month</span>
            <b className={over ? 'ccm-up' : undefined}>{money(projected)}</b>
          </div>
        </div>
        <div className="ccm-trend-chart">
          <Chart height="100%" option={chart(months, amount, actual, forecast)} />
        </div>
        <div className="ccm-amounts ccm-amounts-view">
          <table>
            <thead>
              <tr>
                <th className="sticky">{(budgetBy(a.metrics_query) || 'BUDGET').toUpperCase()}</th>
                <th className="num sticky2">BUDGET</th>
                <th className="sticky3">SPENT</th>
                {months.map((m) => (
                  <th key={m} className="num">
                    {monthLabel(m).toUpperCase()}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => {
                const budget = r.entries.reduce((s, e) => s + e.amount, 0);
                const cost = r.entries.reduce((s, e) => s + (e.costs?.actual ?? 0), 0);
                return (
                  <tr key={r.name}>
                    <td className="sticky">{r.name}</td>
                    <td className="num sticky2">{money(budget)}</td>
                    <td className="sticky3">
                      <div className="ccm-coverage">
                        <ShareBar share={budget ? cost / budget : 0} tone={cost > budget ? 'red' : 'blue'} />
                        <span>{money(cost)}</span>
                      </div>
                    </td>
                    {months.map((m) => {
                      const e = r.entries.find((x) => x.month === m);
                      const act = e?.costs?.actual;
                      return (
                        <td key={m} className="num">
                          <div>{money(e?.amount ?? 0)}</div>
                          {act != null && <small className={act > (e?.amount ?? 0) ? 'ccm-up' : 'muted-text'}>{money(act)}</small>}
                        </td>
                      );
                    })}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        <p className="muted-text ccm-hint">Each month shows its budget and, below it, what it cost so far.</p>
      </div>
    </div>
  );
}

function budgetBy(q: string): string {
  return /by\s*\{([^}]*)\}/.exec(q)?.[1].split(',').join(' / ') ?? '';
}
