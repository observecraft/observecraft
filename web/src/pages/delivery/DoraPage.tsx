import { useMemo, useState } from 'react';
import type { EChartsOption } from 'echarts';
import type { DoraResponse, DoraSummary } from '../../api/ci/client';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { fmtAgo, fmtDur } from './common';
import { DoraIntro } from './DoraIntro';
import { useDora } from './queries';

const RANGES = [
  { value: '7', label: 'Past 7 days' },
  { value: '14', label: 'Past 14 days' },
  { value: '30', label: 'Past 30 days' },
  { value: '90', label: 'Past 90 days' },
];
const RATING_LABEL: Record<string, string> = {
  elite: 'Elite',
  high: 'High',
  medium: 'Medium',
  low: 'Low',
};

/** The four DORA metrics from deployment jobs, commits and incidents. */
export function DoraPage() {
  const [days, setDays] = useState('7');
  const [env, setEnv] = useState('production');
  const [service, setService] = useState('*');
  // Range is fixed per selection so the query key stays stable.
  const range = useMemo(() => {
    const to = Date.now();
    return { from: to - Number(days) * 86_400_000, to };
  }, [days]);
  const q = useDora(range, env, service === '*' ? [] : [service]);
  const all = useDora(range, env, []);
  const d = q.data;
  if (d && d.environments.length === 0) return <DoraIntro />;

  return (
    <div className="ci-dora">
      <div className="ci-overview-head">
        <div>
          <h2>DORA Metrics</h2>
          <p className="muted">Deployments come from CI deploy jobs ({'deployment.environment.name'}); incidents from Incident Response.</p>
        </div>
        <div className="ci-dora-filters">
          <Select label="Environment" value={env} onChange={setEnv} options={(d?.environments.length ? d.environments : ['production']).map((e) => ({ value: e, label: e }))} />
          <Select
            label="Service"
            value={service}
            onChange={setService}
            options={[
              { value: '*', label: 'All services' },
              ...(all.data?.services ?? []).map((s) => ({
                value: s.service,
                label: s.service,
              })),
            ]}
          />
          <Select label="Timeframe" value={days} onChange={setDays} options={RANGES} />
        </div>
      </div>
      {q.isPending ? (
        <Loading label="Computing DORA metrics…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <div className="ci-dora-grid">
            <Metric
              title="Deployment Frequency"
              value={`${d!.summary.deploymentsPerDay} / day`}
              sub={`${(d!.summary.deployments - d!.summary.failedDeployments).toLocaleString()} successful deployments`}
              rating={d!.summary.ratings.deploymentFrequency}
              chart={bars(d!, 'deployments')}
            />
            <Metric
              title="Lead Time for Changes"
              value={fmtDur(d!.summary.leadTimeMs)}
              sub="Median time from commit to running in this environment"
              rating={d!.summary.ratings.leadTime}
              chart={leadTime(d!)}
            />
            <Metric
              title="Change Failure Rate"
              value={d!.summary.changeFailureRate === null || d!.summary.changeFailureRate === undefined ? '—' : `${d!.summary.changeFailureRate}%`}
              sub={`${d!.summary.failedDeployments} failed deployments; successful ones followed by an incident within a day also count`}
              rating={d!.summary.ratings.changeFailureRate}
              chart={bars(d!, 'failed')}
            />
            <Metric
              title="Time to Restore Service"
              value={fmtDur(d!.summary.timeToRestoreMs)}
              sub={d!.incidents ? `Median resolution of ${d!.incidents} incident${d!.incidents === 1 ? '' : 's'} on these services` : 'No resolved incidents on these services in this timeframe'}
              rating={d!.summary.ratings.timeToRestore}
            />
          </div>
          <section className="ci-card">
            <h3>By service</h3>
            <table className="ci-table">
              <thead>
                <tr>
                  <th>SERVICE</th>
                  <th>REPOSITORY</th>
                  <th className="num">DEPLOYMENTS</th>
                  <th className="num">PER DAY</th>
                  <th className="num">LEAD TIME</th>
                  <th className="num">CHANGE FAILURE RATE</th>
                  <th className="num">TIME TO RESTORE</th>
                  <th>LAST DEPLOYMENT</th>
                </tr>
              </thead>
              <tbody>
                {d!.services.map((s) => (
                  <tr key={s.service} onClick={() => setService(s.service)}>
                    <td className="ci-strong">{s.service}</td>
                    <td>{s.repository}</td>
                    <td className="num">{s.summary.deployments}</td>
                    <td className="num">
                      {s.summary.deploymentsPerDay} <Rating r={s.summary.ratings.deploymentFrequency} small />
                    </td>
                    <td className="num">
                      {fmtDur(s.summary.leadTimeMs)} <Rating r={s.summary.ratings.leadTime} small />
                    </td>
                    <td className="num">
                      {pct(s.summary)} <Rating r={s.summary.ratings.changeFailureRate} small />
                    </td>
                    <td className="num">{fmtDur(s.summary.timeToRestoreMs)}</td>
                    <td>{fmtAgo(s.lastDeploy)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {d!.services.length === 0 && <p className="mon-empty">No deployments to {env} in this timeframe.</p>}
          </section>
        </>
      )}
    </div>
  );
}

function pct(s: DoraSummary) {
  return s.changeFailureRate === null || s.changeFailureRate === undefined ? '—' : `${s.changeFailureRate}%`;
}

function Rating({ r, small }: { r?: string; small?: boolean }) {
  if (!r) return null;
  return <span className={`ci-rating r-${r}${small ? ' small' : ''}`}>{RATING_LABEL[r]}</span>;
}

function Metric({ title, value, sub, rating, chart }: { title: string; value: string; sub: string; rating?: string; chart?: EChartsOption }) {
  return (
    <section className="ci-card ci-metric">
      <h3>
        {title} <Rating r={rating} />
      </h3>
      <b className="ci-metric-value">{value}</b>
      <p className="muted">{sub}</p>
      {chart && <Chart option={chart} height={120} />}
    </section>
  );
}

function bars(d: DoraResponse, k: 'deployments' | 'failed'): EChartsOption {
  return {
    grid: { left: 30, right: 6, top: 8, bottom: 20 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'time' },
    yAxis: { type: 'value', minInterval: 1, splitNumber: 2 },
    series: [
      {
        type: 'bar',
        name: k === 'failed' ? 'Failed' : 'Deployments',
        barMaxWidth: 16,
        color: k === 'failed' ? '#e5383b' : '#6a3fd1',
        data: d.days.map((x) => [x.day + 43_200_000, k === 'failed' ? x.failed : x.deployments - x.failed]),
      },
    ],
  };
}

function leadTime(d: DoraResponse): EChartsOption {
  return {
    grid: { left: 60, right: 6, top: 8, bottom: 20 },
    tooltip: { trigger: 'axis', valueFormatter: (v) => fmtDur(v as number) },
    xAxis: { type: 'time' },
    yAxis: {
      type: 'value',
      splitNumber: 2,
      axisLabel: { formatter: (v: number) => fmtDur(v) },
    },
    series: [
      {
        type: 'line',
        name: 'Median lead time',
        color: '#29a8a0',
        connectNulls: true,
        data: d.days.map((x) => [x.day + 43_200_000, x.leadTimeMs ?? null]),
      },
    ],
  };
}
