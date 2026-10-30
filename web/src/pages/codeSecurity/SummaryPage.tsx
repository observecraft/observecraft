import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowRight, ChevronDown, ChevronUp, GitBranch, Globe, LayoutDashboard, Rocket, ShieldAlert, ShieldCheck, Wrench } from 'lucide-react';
import { summary, type CodeSecuritySummary, type SeverityCounts } from '../../api/codeSecurity/client';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { token } from '../../theme/chartTheme';
import { SeverityBadge, shortRepo } from './common';

const NAV = [
  { id: 'posture', label: 'Security posture' },
  { id: 'libraries', label: 'Library vulnerabilities' },
  { id: 'code', label: 'Code vulnerabilities' },
  { id: 'secrets', label: 'Secrets' },
];

function history(s: CodeSecuritySummary): EChartsOption {
  const muted = token('--text-muted');
  return {
    grid: { left: 8, right: 8, top: 16, bottom: 30, containLabel: true },
    tooltip: { trigger: 'axis' },
    legend: { bottom: 0, left: 0, textStyle: { color: muted } },
    xAxis: { type: 'category', data: s.history.map((d) => new Date(d.day).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })), axisLabel: { color: muted }, axisTick: { show: false } },
    yAxis: { type: 'value', minInterval: 1, axisLabel: { color: muted } },
    series: [
      { name: 'Library vulnerabilities', type: 'bar', stack: 'v', data: s.history.map((d) => d.libraries), color: token('--chart-1'), barMaxWidth: 40 },
      { name: 'Code vulnerabilities', type: 'bar', stack: 'v', data: s.history.map((d) => d.code), color: token('--chart-4'), barMaxWidth: 40 },
      { name: 'Secrets', type: 'bar', stack: 'v', data: s.history.map((d) => d.secrets), color: token('--chart-5'), barMaxWidth: 40 },
    ],
  };
}

function Counts({ title, counts, to }: { title: string; counts: SeverityCounts; to: string }) {
  const total = counts.critical + counts.high + counts.medium + counts.low + counts.unknown;
  return (
    <Link className="cs-count-card" to={to}>
      <span className="cs-count-title">{title}</span>
      <b>{total}</b>
      <span className="cs-count-sevs">
        {(['critical', 'high', 'medium', 'low'] as const).map((k) => (
          <span key={k} className={`cs-sev-text-${k}`} title={k}>
            {counts[k]} {k[0].toUpperCase()}
          </span>
        ))}
      </span>
    </Link>
  );
}

/** Code Security → Summary: posture, trends and where the vulnerabilities are. */
export function SummaryPage() {
  const q = useQuery({ queryKey: ['code-security', 'summary'], queryFn: ({ signal }) => summary(signal) });
  const [more, setMore] = useState(false);
  const [groupBy, setGroupBy] = useState<'service' | 'repository'>('service');
  const option = useMemo(() => (q.data ? history(q.data) : null), [q.data]);
  if (q.isPending) return <Loading label="Loading Code Security…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const s = q.data;
  const impacted = s.emerging.filter((a) => a.impacted);
  const emerging = more ? s.emerging : s.emerging.slice(0, 1);
  const groups = groupBy === 'service' ? s.byService : s.byRepository;
  const libTotal = s.libraries.critical + s.libraries.high + s.libraries.medium + s.libraries.low + s.libraries.unknown;
  const jump = (id: string) => document.getElementById(`cs-${id}`)?.scrollIntoView({ behavior: 'smooth' });
  return (
    <div className="cs-summary">
      <aside className="cs-side">
        <div className="cs-side-head">
          <LayoutDashboard size={15} /> Summary
        </div>
        {NAV.map((n) => (
          <button key={n.id} onClick={() => jump(n.id)}>
            {n.label}
          </button>
        ))}
      </aside>
      <div className="cs-summary-main">
        <h1>Code Security</h1>
        <p className="muted">Track vulnerabilities in your libraries and code, monitor progress, and act on what reaches production.</p>

        <h2 id="cs-posture">Security posture</h2>
        <section className="cs-card cs-emerging">
          <div className="cs-emerging-icon">
            <ShieldAlert size={22} />
            <span>Emerging Vulnerabilities</span>
          </div>
          <div className="cs-emerging-body">
            <div className="cs-emerging-head">
              <h3>
                {s.emerging.length === 0
                  ? 'No emerging vulnerabilities for the libraries you use'
                  : impacted.length
                    ? `You are impacted by ${impacted.length} of ${s.emerging.length} emerging vulnerabilities`
                    : `You are not impacted by ${s.emerging.length} emerging vulnerabilities`}
              </h3>
              <Link to="/code-security/detection-coverage/cves" className="link-btn">
                View CVE Explorer <ArrowRight size={14} />
              </Link>
            </div>
            <div className="cs-emerging-scope">
              <span className="muted">Analyzed</span>
              <span className="cs-chip">
                <GitBranch size={13} /> {s.repositoriesAnalyzed} {s.repositoriesAnalyzed === 1 ? 'repository' : 'repositories'}
              </span>
              <span className="cs-chip">
                <Globe size={13} /> {s.servicesAnalyzed} {s.servicesAnalyzed === 1 ? 'service' : 'services'}
              </span>
            </div>
            {emerging.map((a) => (
              <div key={a.id} className="cs-emerging-row">
                <SeverityBadge severity={a.severity} />
                <Link to={`/code-security/detection-coverage/cves?advisory=${encodeURIComponent(a.id)}`} className="cs-mono">
                  {a.cve ?? a.id}
                </Link>
                <span className="ellipsis">{a.summary}</span>
                <span className={a.impacted ? 'cs-impacted' : 'muted'}>{a.impacted ? 'Impacted' : 'Not impacted'}</span>
              </div>
            ))}
            {s.emerging.length > 1 && (
              <button className="link-btn" onClick={() => setMore((m) => !m)}>
                {more ? 'Show less' : `Show ${s.emerging.length - 1} more emerging vulnerabilities`} {more ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
              </button>
            )}
          </div>
        </section>

        <section className="cs-card">
          <h3>Open vulnerabilities and secrets over time</h3>
          {option && <Chart option={option} height={260} />}
        </section>

        <h2 id="cs-libraries">Library vulnerabilities</h2>
        <div className="cs-two">
          <section className="cs-card">
            <h3>Vulnerabilities from default branch to production</h3>
            <p className="muted">Fix first the vulnerabilities that run in production and have a fixed version.</p>
            <div className="cs-funnel">
              {[
                { icon: ShieldCheck, label: 'Open', value: libTotal, to: '/code-security/vulnerabilities/libraries' },
                { icon: GitBranch, label: 'In repositories', value: s.inRepositories, to: '/code-security/vulnerabilities/libraries' },
                { icon: Rocket, label: 'In production', value: s.inProduction, to: '/code-security/vulnerabilities/libraries?scope=production' },
                { icon: Wrench, label: 'Fix available', value: s.fixable, to: '/code-security/vulnerabilities/libraries' },
              ].map((step, i) => (
                <Link key={step.label} to={step.to} className="cs-funnel-step" style={{ ['--h' as string]: `${libTotal ? Math.max(18, (step.value / libTotal) * 100) : 18}%` }}>
                  <span className="cs-funnel-label">
                    <step.icon size={14} /> {step.label}
                  </span>
                  <b>{step.value}</b>
                  <i data-i={i} />
                </Link>
              ))}
            </div>
          </section>
          <section className="cs-card">
            <div className="cs-card-head">
              <h3>Library vulnerabilities grouped</h3>
              <div className="apm-seg">
                <button className={groupBy === 'service' ? 'active' : ''} onClick={() => setGroupBy('service')}>
                  By Service
                </button>
                <button className={groupBy === 'repository' ? 'active' : ''} onClick={() => setGroupBy('repository')}>
                  By Repository
                </button>
              </div>
            </div>
            <p className="muted">Assets with the most open vulnerabilities.</p>
            <table className="cs-table">
              <thead>
                <tr>
                  <th>{groupBy === 'service' ? 'SERVICE' : 'REPOSITORY'}</th>
                  <th className="num">CRITICAL</th>
                  <th className="num">HIGH</th>
                  <th className="num">TOTAL</th>
                </tr>
              </thead>
              <tbody>
                {groups.map((g) => (
                  <tr key={g.name}>
                    <td>
                      <Link to={`/code-security/vulnerabilities/libraries?${groupBy}=${encodeURIComponent(g.name)}`}>{groupBy === 'service' ? g.name : shortRepo(g.name)}</Link>
                    </td>
                    <td className="num cs-sev-text-critical">{g.critical}</td>
                    <td className="num cs-sev-text-high">{g.high}</td>
                    <td className="num">{g.count}</td>
                  </tr>
                ))}
                {groups.length === 0 && (
                  <tr>
                    <td colSpan={4} className="muted">
                      No open library vulnerabilities.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </section>
        </div>

        <h2 id="cs-code">Code vulnerabilities</h2>
        <div className="cs-counts">
          <Counts title="Libraries (SCA)" counts={s.libraries} to="/code-security/vulnerabilities/libraries" />
          <Counts title="Static Code (SAST)" counts={s.code} to="/code-security/vulnerabilities/static-code" />
          <Counts title="Runtime Code (IAST)" counts={s.runtimeCode} to="/code-security/vulnerabilities/runtime-code" />
          <Counts title="Infrastructure as Code" counts={s.iac} to="/code-security/vulnerabilities/iac" />
        </div>

        <h2 id="cs-secrets">Secrets</h2>
        <div className="cs-counts">
          <Counts title="Secrets in repositories" counts={s.secrets} to="/code-security/vulnerabilities/secrets" />
        </div>
      </div>
    </div>
  );
}
