import { useEffect, useMemo, useState } from 'react';
import type { EChartsOption } from 'echarts';
import { Link, useSearchParams } from 'react-router-dom';
import { ChevronDown, ChevronRight, Download, PanelLeftClose, PanelLeftOpen, Plus, Search, Settings, Siren, User } from 'lucide-react';
import { SEVERITIES, SEVERITY_LABEL, type IncidentStatus, type IncidentSummary } from '../../api/incidents/client';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { statusColor, token } from '../../theme/chartTheme';
import { useTeams } from '../settings/queries';
import { DeclareModal } from './DeclareModal';
import { durationLong, durationShort, fmtDateTime, useIncidents } from './queries';
import '../settings/settings.css';
import './incidents.css';

const DAY = 86_400_000;
const RANGES = [
  { value: '7', label: 'Past 7 Days', short: '1w' },
  { value: '30', label: 'Past 30 Days', short: '1mo' },
  { value: '90', label: 'Past 3 Months', short: '3mo' },
  { value: '365', label: 'Past 1 Year', short: '1y' },
];
const STATUSES: { id: IncidentStatus; label: string }[] = [
  { id: 'active', label: 'Active' },
  { id: 'stable', label: 'Stable' },
  { id: 'resolved', label: 'Resolved' },
];

export function StatusBadge({ status }: { status: IncidentStatus }) {
  return <span className={`inc-status inc-${status}`}>{status.toUpperCase()}</span>;
}

function toggle<T>(list: T[], v: T): T[] {
  return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
}

function Accordion({ title, children, defaultOpen = false }: { title: string; children: React.ReactNode; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="inc-acc">
      <button className="inc-acc-title" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />} {title}
      </button>
      {open && <div className="inc-acc-body">{children}</div>}
    </div>
  );
}

function csv(rows: IncidentSummary[]) {
  const esc = (s: string) => `"${s.replace(/"/g, '""')}"`;
  const lines = [['id', 'title', 'status', 'severity', 'type', 'commander', 'declared', 'resolved'].join(',')].concat(
    rows.map((r) =>
      [`IR-${r.id}`, r.title, r.status, r.severity, r.incidentType, r.commander?.name ?? '', new Date(r.declaredAt).toISOString(), r.resolvedAt ? new Date(r.resolvedAt).toISOString() : '']
        .map(esc)
        .join(','),
    ),
  );
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([lines.join('\n')], { type: 'text/csv' }));
  a.download = 'incidents.csv';
  a.click();
  URL.revokeObjectURL(a.href);
}

export function IncidentsPage() {
  const [tab, setTab] = useState<'list' | 'analytics'>('list');
  const [query, setQuery] = useState('');
  const [mine, setMine] = useState(false);
  const [statuses, setStatuses] = useState<IncidentStatus[]>(['active', 'stable', 'resolved']);
  const [severities, setSeverities] = useState<string[]>([]);
  const [teams, setTeams] = useState<string[]>([]);
  const [services, setServices] = useState<string[]>([]);
  const [detectionMethods, setDetectionMethods] = useState<string[]>([]);
  const [incidentTypes, setIncidentTypes] = useState<string[]>([]);
  const [customerImpact, setCustomerImpact] = useState<boolean | null>(null);
  const [days, setDays] = useState('365');
  const [controls, setControls] = useState(true);
  const [declaring, setDeclaring] = useState(false);
  // ?declare=1 (the sidebar's "Declare Incident") opens the declare dialog once.
  const [params, setParams] = useSearchParams();
  const wantsDeclare = params.get('declare') === '1';
  useEffect(() => {
    if (!wantsDeclare) return;
    setDeclaring(true);
    setParams({}, { replace: true });
  }, [wantsDeclare, setParams]);
  // Range is fixed per selection so the query key stays stable while the page is open.
  const [to] = useState(() => Date.now());
  const range = useMemo(() => ({ from: to - Number(days) * DAY, to }), [days, to]);
  const allTeams = useTeams({ query: '', sort: 'name', offset: 0, limit: 200 });

  const q = useIncidents({ query, statuses: statuses.length === 3 ? [] : statuses, severities, mine, teams, services, detectionMethods, incidentTypes, customerImpact, range, offset: 0, limit: 200 });
  const live = (q.data?.incidents ?? []).filter((i) => i.status !== 'resolved');
  const resolved = (q.data?.incidents ?? []).filter((i) => i.status === 'resolved');
  const rangeOpt = RANGES.find((r) => r.value === days)!;

  return (
    <div className="inc-page">
      <header className="product-header">
        <div className="product-title">
          <Siren size={20} /> Incidents
        </div>
        <nav className="product-tabs">
          <button className={`product-tab${tab === 'list' ? ' active' : ''}`} onClick={() => setTab('list')}>
            List
          </button>
          <button className={`product-tab${tab === 'analytics' ? ' active' : ''}`} onClick={() => setTab('analytics')}>
            Analytics
          </button>
        </nav>
        <div className="inc-header-actions">
          <button className="btn" disabled title="Incident settings (types, severities, notification rules) are not configurable yet">
            <Settings size={14} /> Settings
          </button>
          <button className="btn btn-primary" onClick={() => setDeclaring(true)}>
            <Plus size={14} /> Declare Incident
          </button>
        </div>
      </header>
      <div className="inc-search">
        <Search size={18} />
        <input placeholder="Search incidents…" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      <div className="inc-body">
        {controls && (
          <aside className="inc-controls">
            <button className={`inc-scope${!mine ? ' active' : ''}`} onClick={() => setMine(false)}>
              <Siren size={18} /> All Incidents
            </button>
            <button className={`inc-scope${mine ? ' active' : ''}`} onClick={() => setMine(true)}>
              <User size={18} /> My Incidents
            </button>
            <div className="inc-box">
              <Accordion title="Teams">
                {(allTeams.data?.teams ?? []).map((t) => (
                  <label key={t.handle} className="inc-check">
                    <input type="checkbox" className="checkbox" checked={teams.includes(t.handle)} onChange={() => setTeams(toggle(teams, t.handle))} />
                    {t.handle}
                    <span className="inc-count">{q.data?.teamCounts[t.handle] ?? 0}</span>
                  </label>
                ))}
              </Accordion>
            </div>
            <div className="inc-group-title">ASSESSMENT</div>
            <Accordion title="Status" defaultOpen>
              {STATUSES.map((s) => (
                <label key={s.id} className="inc-check">
                  <input type="checkbox" className="checkbox" checked={statuses.includes(s.id)} onChange={() => setStatuses(toggle(statuses, s.id))} />
                  <i className={`inc-dot inc-bg-${s.id}`} />
                  {s.label}
                  <span className="inc-count">{q.data?.counts[s.id] ?? '–'}</span>
                </label>
              ))}
            </Accordion>
            <Accordion title="Severity">
              {SEVERITIES.map((s) => (
                <label key={s} className="inc-check">
                  <input type="checkbox" className="checkbox" checked={severities.includes(s)} onChange={() => setSeverities(toggle(severities, s))} />
                  {s} <span className="muted">{SEVERITY_LABEL[s]}</span>
                  <span className="inc-count">{q.data?.severityCounts[s] ?? 0}</span>
                </label>
              ))}
            </Accordion>
            <div className="inc-group-title">IMPACT</div>
            <Accordion title="Customer Impact">
              {(['yes', 'no'] as const).map((v) => (
                <label key={v} className="inc-check">
                  <input type="checkbox" className="checkbox" checked={customerImpact === (v === 'yes')} onChange={() => setCustomerImpact((c) => (c === (v === 'yes') ? null : v === 'yes'))} />
                  {v === 'yes' ? 'Customers impacted' : 'No customer impact'}
                  <span className="inc-count">{q.data?.impactCounts[v] ?? 0}</span>
                </label>
              ))}
            </Accordion>
            <Accordion title="Services">
              <FacetChecks counts={q.data?.serviceCounts} selected={services} onToggle={(v) => setServices(toggle(services, v))} />
            </Accordion>
            <div className="inc-group-title">DETECTION</div>
            <Accordion title="Detection Method">
              <FacetChecks counts={q.data?.detectionCounts} selected={detectionMethods} onToggle={(v) => setDetectionMethods(toggle(detectionMethods, v))} />
            </Accordion>
            <Accordion title="Incident Type">
              <FacetChecks counts={q.data?.typeCounts} selected={incidentTypes} onToggle={(v) => setIncidentTypes(toggle(incidentTypes, v))} />
            </Accordion>
          </aside>
        )}
        <main className="inc-main">
          <div className="inc-toolbar">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={16} /> : <PanelLeftOpen size={16} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <div className="inc-toolbar-right">
              <Select
                value={days}
                onChange={setDays}
                options={RANGES.map((r) => ({ value: r.value, label: r.label }))}
                trigger={() => (
                  <span className="inc-range">
                    <span className="time-short">{rangeOpt.short}</span>
                    {fmtDateTime(range.from)} – {fmtDateTime(range.to)} <ChevronDown size={14} />
                  </span>
                )}
                triggerClassName="inc-range-btn"
              />
              <button className="btn" onClick={() => q.data && csv(q.data.incidents)} disabled={!q.data}>
                <Download size={14} /> Export
              </button>
            </div>
          </div>
          {q.isPending ? (
            <Loading label="Loading incidents…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : tab === 'analytics' ? (
            <Analytics incidents={q.data.incidents} from={range.from} to={range.to} />
          ) : (
            <>
              <Group title={live.length === 1 ? 'There is 1 live incident.' : `There are ${live.length} live incidents.`} items={live} />
              <Group title={`Resolved incidents (${resolved.length})`} items={resolved} />
              {q.data.total > q.data.incidents.length && (
                <p className="muted inc-more">
                  Showing the {q.data.incidents.length} most relevant of {q.data.total}. Narrow the filters to see more.
                </p>
              )}
            </>
          )}
        </main>
      </div>
      {declaring && <DeclareModal onClose={() => setDeclaring(false)} />}
    </div>
  );
}

function Group({ title, items }: { title: string; items: IncidentSummary[] }) {
  const [open, setOpen] = useState(true);
  if (items.length === 0 && title.startsWith('Resolved')) return null;
  return (
    <section className="inc-group">
      <button className="inc-group-head" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={20} /> : <ChevronRight size={20} />} {title}
      </button>
      {open && items.map((i) => <IncidentCard key={i.id} i={i} />)}
    </section>
  );
}

const DETECTION_LABEL: Record<string, string> = { monitor: 'monitor', customer: 'customer', employee: 'employee', automation: 'automation', unknown: 'unknown' };

function IncidentCard({ i }: { i: IncidentSummary }) {
  const since = i.status === 'resolved' && i.resolvedAt ? i.resolvedAt - i.declaredAt : Date.now() - i.declaredAt;
  return (
    <Link to={`/incidents/${i.id}`} className={`inc-card inc-border-${i.status}`}>
      <div className="inc-card-top">
        <h3>
          IR-{i.id}: {i.test && '[TEST] '}
          <b>{i.title}</b>
        </h3>
        <span className="inc-sev">{i.severity}</span>
        <StatusBadge status={i.status} />
      </div>
      <div className="inc-card-meta">
        <span>
          Commander:{' '}
          {i.commander ? (
            <span className="person-chip">
              <UserAvatar id={i.commander.id} name={i.commander.name} size={18} /> {i.commander.name}
            </span>
          ) : (
            <span className="muted">Unassigned</span>
          )}
        </span>
        <span className="inc-sep" />
        <span>
          Incident Type: <b>{i.incidentType}</b>
        </span>
      </div>
      <div className="inc-card-props">
        <div>
          <div className="muted">Detection Method</div>
          <span className="prop-chip">{DETECTION_LABEL[i.detectionMethod] ?? i.detectionMethod}</span>
        </div>
        <div>
          <div className="muted">State Category</div>
          <span className="prop-chip">{i.status === 'resolved' ? 'resolved' : 'live'}</span>
        </div>
        {i.services.length > 0 && (
          <div>
            <div className="muted">Services</div>
            {i.services.map((s) => (
              <span key={s} className="prop-chip">
                {s}
              </span>
            ))}
          </div>
        )}
      </div>
      <div className="inc-card-foot">
        <span className="age-chip" title={durationLong(since)}>
          {durationShort(since)}
        </span>
        {i.status === 'resolved' && i.resolvedAt ? `Resolved: ${fmtDateTime(i.resolvedAt)}` : `Since: ${fmtDateTime(i.declaredAt)}`}
      </div>
    </Link>
  );
}

function Analytics({ incidents, from, to }: { incidents: IncidentSummary[]; from: number; to: number }) {
  const option = useMemo<EChartsOption>(() => {
    const week = 7 * DAY;
    const start = Math.floor(from / week) * week;
    const buckets: number[] = [];
    for (let t = start; t <= to; t += week) buckets.push(t);
    const sevs = SEVERITIES.filter((s) => incidents.some((i) => i.severity === s));
    const palette = ['--status-error', '--status-warn', '--chart-4', '--chart-1', '--chart-3', '--status-debug'];
    return {
      tooltip: { trigger: 'axis' },
      legend: { bottom: 0 },
      grid: { left: 40, right: 16, top: 16, bottom: 48 },
      xAxis: { type: 'time' },
      yAxis: { type: 'value', minInterval: 1 },
      series: sevs.map((s) => ({
        name: s,
        type: 'bar',
        stack: 'sev',
        barMaxWidth: 18,
        itemStyle: { color: token(palette[SEVERITIES.indexOf(s as (typeof SEVERITIES)[number])]) },
        data: buckets.map((b) => [b, incidents.filter((i) => i.severity === s && i.declaredAt >= b && i.declaredAt < b + week).length]),
      })),
    };
  }, [incidents, from, to]);
  const resolved = incidents.filter((i) => i.resolvedAt);
  const mttr = resolved.length ? resolved.reduce((a, i) => a + (i.resolvedAt! - i.declaredAt), 0) / resolved.length : 0;
  const mttd = incidents.length ? incidents.reduce((a, i) => a + (i.declaredAt - i.detectedAt), 0) / incidents.length : 0;
  return (
    <div className="inc-analytics">
      <div className="inc-tiles">
        <div className="settings-card">
          <div className="muted">Incidents</div>
          <b>{incidents.length}</b>
        </div>
        <div className="settings-card">
          <div className="muted">Open now</div>
          <b style={{ color: statusColor('error') }}>{incidents.filter((i) => i.status !== 'resolved').length}</b>
        </div>
        <div className="settings-card">
          <div className="muted">Mean time to resolve</div>
          <b>{resolved.length ? durationShort(mttr) : '—'}</b>
        </div>
        <div className="settings-card">
          <div className="muted">Mean time to declare</div>
          <b>{incidents.length ? durationShort(mttd) : '—'}</b>
        </div>
        <div className="settings-card">
          <div className="muted">With customer impact</div>
          <b>{incidents.filter((i) => i.customerImpact).length}</b>
        </div>
      </div>
      <section className="settings-card">
        <h2>Incidents per week by severity</h2>
        <Chart option={option} height={300} />
      </section>
    </div>
  );
}

/** Checkboxes for a facet's values, most incidents first (selected values stay listed). */
function FacetChecks({ counts, selected, onToggle }: { counts?: Record<string, number>; selected: string[]; onToggle: (value: string) => void }) {
  const values = [...new Set([...selected, ...Object.keys(counts ?? {})])].sort((a, b) => (counts?.[b] ?? 0) - (counts?.[a] ?? 0) || a.localeCompare(b));
  if (!values.length) return <span className="muted inc-check">None</span>;
  return (
    <>
      {values.map((v) => (
        <label key={v} className="inc-check">
          <input type="checkbox" className="checkbox" checked={selected.includes(v)} onChange={() => onToggle(v)} />
          {v}
          <span className="inc-count">{counts?.[v] ?? 0}</span>
        </label>
      ))}
    </>
  );
}
