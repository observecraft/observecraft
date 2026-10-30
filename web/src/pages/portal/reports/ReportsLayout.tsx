import { useMemo, useState } from 'react';
import { Link, NavLink, Outlet, useLocation, useOutletContext } from 'react-router-dom';
import { BarChart3, ClipboardCheck, ExternalLink, HeartPulse, LayoutDashboard, Pin, PinOff, Plus, Settings, ShieldCheck, Users } from 'lucide-react';
import { Select } from '../../../components/ui/Dropdown';
import { useOnCallTeams, useReportDashboards } from '../catalog/queries';
import { AddDashboardsModal } from './AddDashboardsModal';
import './reports.css';

const DAY = 86_400_000;
const RANGES = [
  { value: '1w', label: 'Past 1 Week', ms: 7 * DAY },
  { value: '1mo', label: 'Past 1 Month', ms: 30 * DAY },
  { value: '3mo', label: 'Past 3 Months', ms: 91 * DAY },
  { value: '6mo', label: 'Past 6 Months', ms: 182 * DAY },
  { value: '1y', label: 'Past 1 Year', ms: 365 * DAY },
];
const PINNED_KEY = 'oc.portal.reports.pinned-team';

export interface ReportsContext {
  range: { from: number; to: number };
  rangeLabel: string;
  /** Selected team handles; empty for every team. */
  teams: string[];
  rangeKey: string;
  setRangeKey: (k: string) => void;
}

/** The report's time frame picker, for the reports that show their own filter bar. */
export function RangeSelect() {
  const { rangeKey, setRangeKey } = useReports();
  return <Select value={rangeKey} options={RANGES.map((r) => ({ value: r.value, label: r.label }))} onChange={setRangeKey} align="right" />;
}

export const useReports = () => useOutletContext<ReportsContext>();

const readPinned = () => {
  try {
    return localStorage.getItem(PINNED_KEY) ?? '';
  } catch {
    return '';
  }
};

/** The reports, the sections each one's navigation jumps to, and whether it shows its own filter bar. */
const REPORTS: { to: string; label: string; icon: typeof Users; sections: [string, string][]; ownFilters?: boolean }[] = [
  { to: 'teams', label: 'Teams Performance', icon: Users, sections: [] },
  {
    to: 'reliability',
    label: 'Reliability Overview',
    icon: ShieldCheck,
    ownFilters: true,
    sections: [
      ['incident-trends', 'Incident Trends'],
      ['slos', 'SLOs'],
    ],
  },
  {
    to: 'catalog-health',
    label: 'Catalog Health',
    icon: HeartPulse,
    ownFilters: true,
    sections: [
      ['summary', 'Summary Information'],
      ['stats', 'Stats on Catalog Entities'],
      ['without-owners', 'Entities Without Owners'],
      ['ownership', 'Entity Ownership Information'],
    ],
  },
  {
    to: 'scorecards',
    label: 'Scorecards Performance',
    icon: ClipboardCheck,
    ownFilters: true,
    sections: [
      ['by-scorecard', 'Performance by Scorecard'],
      ['scorecard-trends', 'Historical Trends by Scorecard'],
      ['by-rule', 'Performance by Rule'],
      ['rule-trends', 'Historical Trends by Rule'],
    ],
  },
];

/** Developer Portal → Reports: the report navigation, the team and time filters, and the selected report. */
export function ReportsLayout() {
  const [rangeKey, setRangeKey] = useState('3mo');
  const [pinned, setPinned] = useState(readPinned);
  const [team, setTeam] = useState(pinned);
  const [adding, setAdding] = useState(false);
  const teams = useOnCallTeams();
  const dashboards = useReportDashboards();
  const ctx = useMemo<ReportsContext>(() => {
    const r = RANGES.find((x) => x.value === rangeKey) ?? RANGES[2];
    // Whole minutes, so queries keep their cache key while the page is open.
    const to = Math.floor(Date.now() / 60_000) * 60_000;
    return { range: { from: to - r.ms, to }, rangeLabel: r.label, teams: team ? [team] : [], rangeKey, setRangeKey };
  }, [rangeKey, team]);
  const location = useLocation();
  const current = REPORTS.find((r) => location.pathname.endsWith(`/${r.to}`));
  const pin = () => {
    const next = pinned === team ? '' : team;
    setPinned(next);
    try {
      if (next) localStorage.setItem(PINNED_KEY, next);
      else localStorage.removeItem(PINNED_KEY);
    } catch {
      /* storage unavailable */
    }
  };

  return (
    <div className="pr-page">
      <aside className="pr-nav" aria-label="Reports">
        <div className="pr-nav-title">Reports</div>
        {REPORTS.map((r) => (
          <div key={r.to}>
            <NavLink to={r.to} className={({ isActive }) => `pr-nav-item${isActive ? ' active' : ''}`}>
              <r.icon size={15} /> {r.label}
            </NavLink>
            {current === r && r.sections.length > 0 && (
              <div className="pr-nav-sections">
                {r.sections.map(([id, label]) => (
                  <button key={id} className="pr-nav-section" onClick={() => document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' })}>
                    {label}
                  </button>
                ))}
              </div>
            )}
          </div>
        ))}
        <Link to="/delivery/dora" className="pr-nav-item">
          <BarChart3 size={15} /> DORA Metrics <ExternalLink size={12} className="pr-nav-ext" />
        </Link>
        <div className="pr-nav-title pr-nav-title-row">
          Custom Reports
          <button className="pr-icon-btn" title="Choose dashboards" aria-label="Choose dashboards" onClick={() => setAdding(true)}>
            <Settings size={14} />
          </button>
        </div>
        {dashboards.data?.length ? (
          dashboards.data.map((d) => (
            <Link key={d.id} to={`/dashboards/${encodeURIComponent(d.id)}`} className="pr-nav-item" title={d.description ?? undefined}>
              <LayoutDashboard size={15} /> <span className="pr-nav-label">{d.title}</span>
            </Link>
          ))
        ) : (
          <div className="pr-empty-card">
            <p>Add dashboards for your organization to view here</p>
            <button className="btn btn-sm" onClick={() => setAdding(true)}>
              <Plus size={13} /> Add Dashboards
            </button>
          </div>
        )}
      </aside>
      <main className="pr-main">
        {!current?.ownFilters && (
          <div className="pr-controls">
            <div className="pr-filter-box">
              <Select label="Teams" value={team} options={[{ value: '', label: 'All teams' }, ...(teams.data ?? []).map((t) => ({ value: t.handle, label: t.name }))]} onChange={setTeam} />
              <button
                className={`pr-icon-btn${pinned && pinned === team ? ' pr-pinned' : ''}`}
                disabled={!team}
                onClick={pin}
                aria-label={pinned === team ? 'Unpin team' : 'Pin team'}
                title="Pin this team as your default"
              >
                {pinned && pinned === team ? <PinOff size={14} /> : <Pin size={14} />}
              </button>
            </div>
            <Select value={rangeKey} options={RANGES.map((r) => ({ value: r.value, label: r.label }))} onChange={setRangeKey} align="right" />
          </div>
        )}
        <Outlet context={ctx} />
      </main>
      {adding && <AddDashboardsModal current={dashboards.data ?? []} onClose={() => setAdding(false)} />}
    </div>
  );
}
