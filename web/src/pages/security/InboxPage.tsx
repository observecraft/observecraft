import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ChevronDown, ChevronRight, Inbox, Search, ThumbsUp } from 'lucide-react';
import type { InboxFinding, SecurityInbox } from '../../api/security/client';
import { Chart } from '../../components/charts/Chart';
import { Select } from '../../components/ui/Dropdown';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact, timeAgo } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { SeverityBadge } from './common';
import { useSecurityInbox } from './queries';

const ALL = '';

/** Security → Inbox: the findings to fix first, with Findings and Dashboard views. */
export function InboxPage({ types }: { types?: string[] } = {}) {
  const [params, setParams] = useSearchParams();
  const view = params.get('view') === 'dashboard' ? 'dashboard' : 'findings';
  const inbox = useSecurityInbox();
  const q = { ...inbox, data: inbox.data && types ? only(inbox.data, types) : inbox.data };
  return (
    <div className="sec-page">
      <nav className="sec-subtabs">
        {(['findings', 'dashboard'] as const).map((v) => (
          <button key={v} className={view === v ? 'active' : ''} onClick={() => setParams(v === 'dashboard' ? { view: v } : {}, { replace: true })}>
            {v === 'findings' ? 'Findings' : 'Dashboard'}
          </button>
        ))}
      </nav>
      <h2 className="sec-h">
        <Inbox size={18} /> Security Inbox <span className="muted sec-h-sub">Your most important security findings, prioritized.</span>
      </h2>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : view === 'findings' ? (
        <Findings data={q.data!} />
      ) : (
        <Dashboard findings={q.data!.findings} />
      )}
    </div>
  );
}

const WEEK = 7 * 86_400_000;

/** The inbox restricted to some finding types (Cloud Security's inbox). */
function only(data: SecurityInbox, types: string[]): SecurityInbox {
  const findings = data.findings.filter((f) => types.includes(f.findingType));
  const weekAgo = Date.now() - WEEK;
  const count = (sev: string, before?: number) => findings.filter((f) => f.severity === sev && (before === undefined || f.firstSeen <= before)).length;
  const byType: Record<string, number> = {};
  for (const f of findings) byType[f.findingType] = (byType[f.findingType] ?? 0) + 1;
  return { findings, byType, critical: count('critical'), high: count('high'), criticalWeekAgo: count('critical', weekAgo), highWeekAgo: count('high', weekAgo) };
}

function Findings({ data }: { data: NonNullable<ReturnType<typeof useSecurityInbox>['data']> }) {
  const [severity, setSeverity] = useState(ALL);
  const [type, setType] = useState(ALL);
  const [service, setService] = useState(ALL);
  const [search, setSearch] = useState('');
  const [open, setOpen] = useState<Set<string>>(new Set());
  const all = data.findings;
  const services = [...new Set(all.flatMap((f) => f.resources))].sort();
  const shown = all.filter(
    (f) =>
      (!severity || f.severity === severity) &&
      (!type || f.findingType === type) &&
      (!service || f.resources.includes(service)) &&
      (!search.trim() || f.title.toLowerCase().includes(search.trim().toLowerCase())),
  );
  const groups = useMemo(() => {
    const m = new Map<string, InboxFinding[]>();
    for (const f of shown) m.set(f.title, [...(m.get(f.title) ?? []), f]);
    return [...m.entries()];
  }, [shown]);
  const change = (now: number, before: number) => (now === before ? '—' : `${now > before ? '+' : ''}${now - before}`);
  return (
    <>
      <div className="sec-filters">
        <Select
          label="Severity"
          value={severity}
          onChange={setSeverity}
          options={[{ value: ALL, label: 'All' }, ...['critical', 'high', 'medium', 'low'].map((s) => ({ value: s, label: s[0].toUpperCase() + s.slice(1) }))]}
        />
        <Select label="Finding Type" value={type} onChange={setType} options={[{ value: ALL, label: 'All' }, ...Object.keys(data.byType).map((t) => ({ value: t, label: t }))]} />
        <Select label="Resource" value={service} onChange={setService} options={[{ value: ALL, label: 'All' }, ...services.map((s) => ({ value: s, label: s }))]} />
        <label className="sec-search small">
          <Search size={14} />
          <input placeholder="Search findings" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
      </div>
      <div className="sec-inbox-cards">
        <section>
          <h4>
            SEVERITY <span className="muted">Change over last week</span>
          </h4>
          <div className="sec-card-row">
            <div className="sec-card">
              <span className="sec-card-change">{change(data.critical, data.criticalWeekAgo)}</span>
              <b>{data.critical}</b> Critical findings
            </div>
            <div className="sec-card">
              <span className="sec-card-change">{change(data.high, data.highWeekAgo)}</span>
              <b>{data.high}</b> High findings
            </div>
          </div>
        </section>
        <section>
          <h4>TRIAGE STATUS</h4>
          <div className="sec-card-row">
            <div className="sec-card">
              <b>{all.filter((f) => f.triage === 'pending').length}</b> Pending triage
            </div>
            <div className="sec-card">
              <b>{all.filter((f) => f.triage === 'in_flight').length}</b> In flight
            </div>
          </div>
        </section>
      </div>
      <div className="mon-table-wrap">
        <div className="sec-table-head">
          <b>{groups.length} Groups</b>
          <span className="muted">Grouped by finding title</span>
        </div>
        {!groups.length ? (
          <div className="sec-empty">
            <ThumbsUp size={40} className="sec-thumb" />
            <h3>You&rsquo;re all caught up with prioritized issues</h3>
            <p className="muted">New prioritized issues show up here as soon as they appear.</p>
          </div>
        ) : (
          <table className="mon-table syn-table sec-inbox">
            <thead>
              <tr>
                <th>FINDING TITLE</th>
                <th>SEVERITY</th>
                <th>TYPE</th>
                <th className="num">RESOURCES</th>
                <th>TRIAGE STATUS</th>
              </tr>
            </thead>
            <tbody>
              {groups.map(([title, items]) => {
                const expanded = open.has(title);
                const resources = new Set(items.flatMap((f) => f.resources));
                return [
                  <tr
                    key={title}
                    className="sec-group-row"
                    onClick={() =>
                      setOpen((s) => {
                        const n = new Set(s);
                        if (n.has(title)) n.delete(title);
                        else n.add(title);
                        return n;
                      })
                    }
                  >
                    <td>
                      {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />} <b>{title}</b> <span className="muted">({items.length})</span>
                    </td>
                    <td>
                      <SeverityBadge severity={items[0].severity} />
                    </td>
                    <td>{items[0].findingType}</td>
                    <td className="num">{resources.size}</td>
                    <td>Pending triage</td>
                  </tr>,
                  ...(expanded
                    ? items.map((f) => (
                        <tr key={f.key} className="sec-finding-row">
                          <td colSpan={2}>
                            <Link to={f.link}>{f.resources.join(', ') || f.title}</Link>
                            <div className="sec-tags">
                              {f.reasons.map((r) => (
                                <span key={r} className="tag-chip">
                                  {r}
                                </span>
                              ))}
                              {f.cve && <span className="tag-chip mono">{f.cve}</span>}
                            </div>
                          </td>
                          <td>{f.resourceType}</td>
                          <td className="num muted">first seen {timeAgo(f.firstSeen)}</td>
                          <td />
                        </tr>
                      ))
                    : []),
                ];
              })}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

/** The inbox's dashboard: counts and breakdowns of the findings. */
function Dashboard({ findings }: { findings: InboxFinding[] }) {
  const count = (sev: string) => findings.filter((f) => f.severity === sev).length;
  const top = (key: (f: InboxFinding) => string[], n = 8) => {
    const m = new Map<string, number>();
    for (const f of findings) for (const k of key(f)) m.set(k, (m.get(k) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1]).slice(0, n);
  };
  const bars = (data: [string, number][], color: string): EChartsOption => ({
    grid: { left: 160, right: 24, top: 8, bottom: 20 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'value', minInterval: 1 },
    yAxis: { type: 'category', inverse: true, data: data.map(([k]) => k), axisLabel: { width: 150, overflow: 'truncate' } },
    series: [{ type: 'bar', barMaxWidth: 14, itemStyle: { color }, data: data.map(([, v]) => v) }],
  });
  const panel = (title: string, data: [string, number][], color = token('--chart-1')) => (
    <div className="sec-dash-widget">
      <h5>{title}</h5>
      {data.length ? <Chart option={bars(data, color)} height={Math.max(120, data.length * 26 + 30)} /> : <p className="muted sec-nodata">(No data)</p>}
    </div>
  );
  return (
    <div className="sec-dashboard">
      <section className="sec-dash-group overview">
        <h3>Overview</h3>
        <div className="sec-dash-grid">
          {(['critical', 'high', 'medium'] as const).map((s) => (
            <div key={s} className="sec-dash-widget value">
              <h5>{s[0].toUpperCase() + s.slice(1)} severity inbox findings</h5>
              <b className={`sev-text-${s}`}>{compact(count(s))}</b>
            </div>
          ))}
          {panel(
            'Inbox findings by type',
            top((f) => [f.findingType]),
          )}
          {panel(
            'Top inbox findings by count',
            top((f) => [f.title]),
            token('--chart-2'),
          )}
          {panel(
            'Inbox findings by resource type',
            top((f) => [f.resourceType]),
            token('--chart-3'),
          )}
          {panel(
            'Top repositories with findings',
            top((f) => f.repositories),
            token('--chart-4'),
          )}
          {panel(
            'Top services with findings',
            top((f) => f.services),
            token('--chart-5'),
          )}
        </div>
      </section>
    </div>
  );
}
