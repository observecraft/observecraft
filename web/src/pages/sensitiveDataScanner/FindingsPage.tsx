// Sensitive Data Scanner → Findings: what the scanner found over the selected time, from the `sds.matches` metric
// (counted per group, rule and service), with links to the matching logs (tagged `sensitive_data:<…>`).
import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { Settings, ShieldAlert } from 'lucide-react';
import { queryTimeseries } from '../../api/dashboards/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact } from '../../lib/format';
import { palette } from '../../theme/chartTheme';
import { useSdsConfiguration } from './queries';
import '../settings/settings.css';
import './sds.css';

const METRIC = 'sds.matches';
const BY = {
  sds_rule: 'Rule',
  service: 'Service',
  sds_group: 'Group',
} as const;
type By = keyof typeof BY;

const logsLink = (query: string) => `/logs?query=${encodeURIComponent(query)}`;
const POINTS = 60;

/** The matches over the selected time by `by`: the API answers counters as per-second rates, so each point is turned
 * back into what its interval counted (rate × interval), and each group gets its total. */
function useMatches(by: By) {
  const { range, refreshVersion } = useTime();
  return useQuery({
    queryKey: ['sds', 'findings', by, range, refreshVersion],
    queryFn: async () => {
      const series = await queryTimeseries({ queries: [`sum:${METRIC}{*} by {${by}}`], range, points: POINTS });
      const times = [...new Set(series.flatMap((s) => s.points.map((p) => p[0])))].sort((a, b) => a - b);
      const gaps = times.slice(1).map((t, i) => t - times[i]);
      const step = (gaps.length ? Math.min(...gaps) : (range.to - range.from) / POINTS) / 1000;
      return series
        .map((s) => {
          const points = s.points.map(([t, v]) => [t, Math.round(v * step)] as [number, number]);
          return { name: s.name.replace(/^\w+:/, ''), points, total: Math.round(s.points.reduce((n, p) => n + p[1] * step, 0)) };
        })
        .filter((s) => s.total > 0)
        .sort((a, b) => b.total - a.total);
    },
    retry: false,
  });
}

function Over({ by, title }: { by: By; title: string }) {
  const { range } = useTime();
  const q = useMatches(by);
  const colors = palette();
  const series = q.data ?? [];
  const option: EChartsOption = {
    grid: { left: 8, right: 12, top: 12, bottom: 40, containLabel: true },
    tooltip: { trigger: 'axis', confine: true },
    legend: { bottom: 0, type: 'scroll' },
    xAxis: { type: 'time', min: range.from, max: range.to },
    yAxis: { type: 'value', minInterval: 1 },
    series: series.map((s, i) => ({ name: s.name, type: 'bar', stack: 'all', color: colors[i % colors.length], data: s.points, barMaxWidth: 14 })),
  };
  return (
    <section className="settings-card sds-chart-card">
      <h2>{title}</h2>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : series.length === 0 ? (
        <p className="muted sds-empty">No matches in this time range.</p>
      ) : (
        <Chart option={option} height={240} />
      )}
    </section>
  );
}

function Top({ by }: { by: By }) {
  const cfg = useSdsConfiguration();
  const q = useMatches(by);
  const rows = q.data ?? [];
  const total = rows.reduce((n, r) => n + r.total, 0);
  /** The tag a rule's matching logs carry, to search them. */
  const ruleQuery = (name: string) => {
    const rule = cfg.data?.groups.flatMap((g) => g.rules).find((r) => r.name === name);
    const tag = rule?.tags.find((t) => t.startsWith('sensitive_data:')) ?? (rule?.standardPatternId ? `sensitive_data:${rule.standardPatternId}` : undefined);
    return tag ?? 'sensitive_data:*';
  };
  return (
    <section className="settings-card">
      <h2>Top {BY[by].toLowerCase()}s</h2>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : rows.length === 0 ? (
        <p className="muted sds-empty">No matches in this time range.</p>
      ) : (
        <table className="settings-table sds-top">
          <thead>
            <tr>
              <th>{BY[by].toUpperCase()}</th>
              <th className="num">MATCHES</th>
              <th className="num">SHARE</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, 10).map((r) => {
              const query = by === 'sds_rule' ? ruleQuery(r.name) : by === 'service' ? `service:${r.name} sensitive_data:*` : 'sensitive_data:*';
              return (
                <tr key={r.name}>
                  <td>{r.name}</td>
                  <td className="num">{compact(r.total)}</td>
                  <td className="num">
                    <span className="sds-bar">
                      <span style={{ width: `${total ? (r.total / total) * 100 : 0}%` }} />
                    </span>
                  </td>
                  <td className="num">
                    <Link className="link-muted" to={logsLink(query)}>
                      View logs
                    </Link>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </section>
  );
}

function Total() {
  const q = useMatches('sds_rule');
  const rows = q.data ?? [];
  const total = rows.reduce((n, r) => n + r.total, 0);
  return (
    <div className="sds-kpis">
      <div className="settings-card sds-kpi">
        <span className="muted">Matches</span>
        <b>{q.isPending ? '…' : q.isError ? '—' : compact(total)}</b>
      </div>
      <div className="settings-card sds-kpi">
        <span className="muted">Rules with matches</span>
        <b>{q.isPending ? '…' : q.isError ? '—' : rows.length}</b>
      </div>
      <div className="settings-card sds-kpi sds-kpi-link">
        <span className="muted">Logs tagged by the scanner</span>
        <Link to={logsLink('sensitive_data:*')}>Search sensitive_data:*</Link>
      </div>
    </div>
  );
}

export function SensitiveDataFindingsPage() {
  return (
    <div className="sds-findings">
      <header className="product-header">
        <div className="product-title">
          <ShieldAlert size={20} /> Sensitive Data Scanner
        </div>
        <div className="sds-header-right">
          <Link className="btn" to="/organization-settings/sensitive-data-scanner">
            <Settings size={14} /> Settings
          </Link>
          <TimePicker compact />
        </div>
      </header>
      <div className="sds-findings-body">
        <p className="muted">
          Matches the scanner found in logs, counted every minute in the <span className="mono">{METRIC}</span> metric (tags <span className="mono">sds_group</span>,{' '}
          <span className="mono">sds_rule</span>, <span className="mono">service</span>), which dashboards and monitors can use too.
        </p>
        <Total />
        <Over by="sds_rule" title="Matches by rule" />
        <div className="sds-two">
          <Top by="sds_rule" />
          <Top by="service" />
        </div>
        <div className="sds-two">
          <Over by="service" title="Matches by service" />
          <Over by="sds_group" title="Matches by group" />
        </div>
      </div>
    </div>
  );
}
