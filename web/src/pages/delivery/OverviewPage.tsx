import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowDownRight, ArrowUpRight } from 'lucide-react';
import type { CiAggregate } from '../../api/ci/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Branch, NO_FILTERS, STATUS_COLOR, fmtAgo, fmtDur } from './common';
import { PipelineDrawer } from './PipelineDrawer';
import { useCharts, useCiOverview } from './queries';

/** CI health at a glance: volume, failure rate, durations, worst pipelines and recent failures. */
export function OverviewPage() {
  const t = useTime();
  const q = useCiOverview(t.range);
  const charts = useCharts({
    range: t.range,
    level: 'pipeline',
    query: '',
    filters: NO_FILTERS,
  });
  const [open, setOpen] = useState<string | null>(null);
  const option = useMemo<EChartsOption | null>(() => {
    const d = charts.data;
    if (!d) return null;
    const sum = (k: 'executions' | 'errored') => d.times.map((_, j) => d.series.reduce((n, s) => n + s[k][j], 0));
    const all = sum('executions');
    const errored = sum('errored');
    return {
      grid: { left: 40, right: 12, top: 30, bottom: 24 },
      legend: { top: 0 },
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'time' },
      yAxis: { type: 'value', minInterval: 1 },
      series: [
        {
          name: 'Succeeded',
          type: 'bar',
          stack: 'x',
          barMaxWidth: 14,
          color: '#3ec46d',
          data: d.times.map((x, j) => [x + d.stepMs / 2, all[j] - errored[j]]),
        },
        {
          name: 'Errored',
          type: 'bar',
          stack: 'x',
          barMaxWidth: 14,
          color: '#e5383b',
          data: d.times.map((x, j) => [x + d.stepMs / 2, errored[j]]),
        },
      ],
    };
  }, [charts.data]);

  if (q.isPending) return <Loading label="Loading CI overview…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const { current: c, previous: p } = q.data;
  const rate = (k: typeof c) => (k.executions ? (k.failures * 100) / k.executions : 0);
  return (
    <div className="ci-overview">
      <div className="ci-overview-head">
        <h2>Pipelines overview</h2>
        <TimePicker />
      </div>
      <div className="ci-kpis">
        <Kpi label="Pipeline executions" value={c.executions.toLocaleString()} now={c.executions} before={p.executions} />
        <Kpi label="Failure rate" value={`${rate(c).toFixed(1)}%`} now={rate(c)} before={rate(p)} lowerIsBetter />
        <Kpi label="Median duration" value={fmtDur(c.p50Ms)} now={c.p50Ms} before={p.p50Ms} lowerIsBetter />
        <Kpi label="p95 duration" value={fmtDur(c.p95Ms)} now={c.p95Ms} before={p.p95Ms} lowerIsBetter />
        <Link to="/ci/flaky" className="ci-kpi link">
          <span>Flaky tests</span>
          <b>{q.data.flakyTests}</b>
          <em>Passed and failed on the same commit</em>
        </Link>
        <div className="ci-kpi">
          <span>Repositories</span>
          <b>{c.repositories}</b>
          <em>reporting pipelines</em>
        </div>
      </div>
      <section className="ci-card">
        <h3>Executions</h3>
        {option ? <Chart option={option} height={220} /> : <Loading />}
      </section>
      <div className="ci-grid2">
        <AggCard title="Most failing pipelines" rows={q.data.topFailing} metric={(a) => `${a.failures} failures · ${((a.failures * 100) / a.executions).toFixed(0)}%`} onOpen={setOpen} />
        <AggCard title="Slowest pipelines (p95)" rows={q.data.slowest} metric={(a) => `${fmtDur(a.p95Ms)} p95 · ${fmtDur(a.p50Ms)} median`} onOpen={setOpen} />
      </div>
      <section className="ci-card">
        <h3>
          Recent failures <Link to="/ci/explorer?q=status:error">View all</Link>
        </h3>
        {q.data.recentFailures.length === 0 ? (
          <p className="muted">No failed pipelines in this timeframe.</p>
        ) : (
          <ul className="ci-fail-list">
            {q.data.recentFailures.map((e) => (
              <li key={e.spanId}>
                <button onClick={() => setOpen(e.traceId)}>
                  <i style={{ background: STATUS_COLOR.error }} />
                  <b>{e.pipelineName}</b>
                  <span>{e.repository}</span>
                  <Branch name={e.branch} />
                  <span className="muted">{e.message}</span>
                  <span className="muted ci-right">
                    {e.author} · {fmtAgo(e.start)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
      {open && <PipelineDrawer traceId={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

function Kpi({ label, value, now, before, lowerIsBetter }: { label: string; value: string; now: number; before: number; lowerIsBetter?: boolean }) {
  const change = before > 0 ? ((now - before) / before) * 100 : null;
  const good = change === null ? null : lowerIsBetter ? change <= 0 : change >= 0;
  return (
    <div className="ci-kpi">
      <span>{label}</span>
      <b>{value}</b>
      {change === null ? (
        <em>not enough history for the previous period</em>
      ) : (
        <em className={good ? 'ci-good' : 'ci-bad'}>
          {change >= 0 ? <ArrowUpRight size={14} /> : <ArrowDownRight size={14} />} {Math.abs(change).toFixed(0)}% vs previous period
        </em>
      )}
    </div>
  );
}

function AggCard({ title, rows, metric, onOpen }: { title: string; rows: CiAggregate[]; metric: (a: CiAggregate) => string; onOpen: (id: string) => void }) {
  return (
    <section className="ci-card">
      <h3>{title}</h3>
      {rows.length === 0 ? (
        <p className="muted">Nothing to show.</p>
      ) : (
        <table className="ci-table">
          <tbody>
            {rows.map((a) => (
              <tr key={`${a.name}|${a.repository}`} onClick={() => onOpen(a.lastTraceId)}>
                <td>
                  <b>{a.name}</b>
                  <div className="muted">{a.repository}</div>
                </td>
                <td className="num">{metric(a)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
