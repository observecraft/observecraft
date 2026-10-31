import { useMemo } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { CheckCircle2, ChevronRight, Circle, ExternalLink } from 'lucide-react';
import type { ContentPackStatus } from '../../api/security/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact, timeAgo } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { PackLogo, SeverityBadge } from './common';
import { useContentPacks, useSiemOverview } from './queries';

const MIN = 60_000;
const PALETTE = ['--chart-1', '--chart-2', '--chart-3', '--chart-4', '--chart-5', '--chart-6', '--chart-7', '--chart-8'];

/** Cloud SIEM → Overview: setup, coverage, logs analyzed, important signals and the log sources. */
export function SiemOverviewPage() {
  const { range } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
  const q = useSiemOverview(r);
  const packs = useContentPacks();
  const o = q.data;
  const chart = useMemo<EChartsOption>(
    () => ({
      grid: { left: 44, right: 12, top: 10, bottom: 40 },
      tooltip: { trigger: 'axis' },
      legend: { bottom: 0, type: 'scroll' },
      xAxis: { type: 'time', min: r.from, max: r.to },
      yAxis: { type: 'value', axisLabel: { formatter: (v: number) => compact(v) } },
      series: (o?.logsAnalyzed ?? []).map((s, i) => ({ name: s.source, type: 'bar', stack: 'logs', barMaxWidth: 16, itemStyle: { color: token(PALETTE[i % PALETTE.length]) }, data: s.points })),
    }),
    [o, r.from, r.to],
  );
  if (q.isLoading) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const steps: [boolean, string, string][] = [
    [o!.setup.packsEnabled > 0, 'Turn on a content pack', '/security/siem/content-packs'],
    [o!.setup.receivingLogs > 0, 'Receive security logs', '/security/siem/content-packs'],
    [o!.setup.rulesActive > 0, 'Detection rules running', '/security/siem/rules'],
    [o!.setup.rulesNotifying > 0, 'Get notified', '/security/siem/rules'],
  ];
  const done = steps.filter(([ok]) => ok).length;
  const categories = o!.coverage.filter((c) => c.active > 0).length;
  return (
    <div className="sec-page siem-overview">
      <div className="sec-explorer-top">
        <div>
          <h2 className="sec-h">Cloud SIEM</h2>
          <p className="muted">Threat detection on your cloud audit, identity, collaboration and web logs.</p>
        </div>
        <span className="syn-grow" />
        <TimePicker compact />
      </div>
      <section className="siem-setup">
        <h4>
          CLOUD SIEM SETUP STATUS <span className={`siem-pct${done === steps.length ? ' full' : ''}`}>{Math.round((done / steps.length) * 100)}%</span>
        </h4>
        <div className="siem-steps">
          {steps.map(([ok, label, to]) => (
            <Link key={label} to={to} className={ok ? 'ok' : ''}>
              {ok ? <CheckCircle2 size={18} /> : <Circle size={18} />} {label}
            </Link>
          ))}
        </div>
        <div className="siem-progress">
          <i style={{ width: `${(done / steps.length) * 100}%` }} />
        </div>
      </section>
      <section className="siem-section">
        <h3>Security Coverage</h3>
        <div className="siem-coverage">
          <div className="siem-cov-main">
            <h5>ENABLED CONTENT PACKS AND LOG SOURCES</h5>
            <p>
              <b>{o!.statuses.active ?? 0}</b> content packs receiving logs. <b>{categories}</b> of <b>{o!.coverage.length}</b> categories covered.
            </p>
            <div className="siem-cov-bar">
              {o!.coverage.map((c, i) => (
                <i key={c.category} title={`${c.category}: ${c.active} of ${c.total}`} style={{ flex: c.total, background: c.active ? `var(${PALETTE[i % PALETTE.length]})` : 'var(--bg-group)' }} />
              ))}
            </div>
            <div className="siem-cov-legend">
              {o!.coverage.map((c, i) => (
                <span key={c.category}>
                  <i style={{ background: c.active ? `var(${PALETTE[i % PALETTE.length]})` : 'var(--bg-group)' }} /> {c.category}
                </span>
              ))}
            </div>
          </div>
          <Link to="/security/siem/content-packs?status=active" className="siem-tile active">
            <span>Active</span>
            <b>{o!.statuses.active ?? 0}</b>
          </Link>
          <Link to="/security/siem/content-packs?status=warning" className="siem-tile warning">
            <span>Warning</span>
            <b>{o!.statuses.warning ?? 0}</b>
          </Link>
          <Link to="/security/siem/content-packs?status=available" className="siem-tile">
            <span>Available</span>
            <b>{o!.statuses.available ?? 0}</b>
          </Link>
        </div>
        <h5 className="siem-h5">LOGS ANALYZED BY CLOUD SIEM</h5>
        {o!.logsAnalyzed.length ? <Chart option={chart} height={240} /> : <p className="muted">No logs of the content packs&rsquo; sources in this time range.</p>}
      </section>
      <section className="siem-section">
        <h3>
          Important Signals <span className="muted">{o!.openSignals} open signals</span>
          <Link className="siem-more" to="/security/siem/signals?state=open">
            View all <ChevronRight size={14} />
          </Link>
        </h3>
        {o!.importantSignals.length ? (
          <ul className="siem-important">
            {o!.importantSignals.map((s) => (
              <li key={s.id}>
                <SeverityBadge severity={s.severity} />
                <Link to={`/security/siem/signals?signal=${s.id}`}>{s.title}</Link>
                <span className="muted">{timeAgo(s.createdAt)}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">No open high or critical signals.</p>
        )}
      </section>
      <section className="siem-section">
        <h3>Log Sources</h3>
        <p className="muted">Security and operational logs together show what happens in your cloud environments.</p>
        {packs.isLoading ? <Loading /> : packs.isError ? <ErrorState error={packs.error} /> : <LogSources packs={packs.data!} />}
      </section>
    </div>
  );
}

function Sparkline({ values }: { values: number[] }) {
  const max = Math.max(1, ...values);
  return (
    <span className="siem-spark" title={`max ${compact(max)} per hour`}>
      {values.map((v, i) => (
        <i key={i} style={{ height: `${Math.max(2, (v / max) * 100)}%` }} />
      ))}
    </span>
  );
}

/** The packs' log sources by platform: installed (sending logs) or available, with their volume. */
export function LogSources({ packs }: { packs: ContentPackStatus[] }) {
  const platforms = [...new Set(packs.map((p) => p.platform))];
  return (
    <table className="mon-table syn-table siem-sources">
      <thead>
        <tr>
          <th>SOURCE</th>
          <th>STATUS</th>
          <th className="num">LOGS INGESTED (24H)</th>
          <th />
        </tr>
      </thead>
      <tbody>
        {platforms.map((pl) => [
          <tr key={pl} className="siem-platform">
            <td colSpan={4}>{pl}</td>
          </tr>,
          ...packs
            .filter((p) => p.platform === pl)
            .map((p) => (
              <tr key={p.id}>
                <td>
                  <span className="siem-source">
                    <PackLogo logo={p.logo} size={20} /> {p.name} <span className="muted mono">{p.query}</span>
                  </span>
                </td>
                <td>
                  <span className={`siem-status ${p.logs24h > 0 ? 'installed' : 'available'}`}>{p.logs24h > 0 ? 'INSTALLED' : 'AVAILABLE'}</span>
                </td>
                <td className="num">
                  {p.logs24h > 0 ? (
                    <span className="siem-volume">
                      <b>{compact(p.logs24h)}</b> <Sparkline values={p.hourly} />
                    </span>
                  ) : (
                    <span className="muted">—</span>
                  )}
                </td>
                <td className="num">
                  {p.logs24h > 0 ? (
                    <Link className="btn btn-ghost" to={`/logs?query=${encodeURIComponent(p.query)}`}>
                      View logs
                    </Link>
                  ) : (
                    <Link className="btn" to={`/security/siem/content-packs?pack=${p.id}`}>
                      Configure Source <ExternalLink size={12} />
                    </Link>
                  )}
                </td>
              </tr>
            )),
        ])}
      </tbody>
    </table>
  );
}
