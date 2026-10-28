import { useMemo, useState } from 'react';
import { Link, NavLink, Outlet, useOutletContext } from 'react-router-dom';
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
}

export const useReports = () => useOutletContext<ReportsContext>();

const readPinned = () => {
  try {
    return localStorage.getItem(PINNED_KEY) ?? '';
  } catch {
    return '';
  }
};

const REPORTS = [
  { to: 'teams', label: 'Teams Performance', icon: Users },
  { to: 'reliability', label: 'Reliability Overview', icon: ShieldCheck },
  { to: 'catalog-health', label: 'Catalog Health', icon: HeartPulse },
  { to: 'scorecards', label: 'Scorecards Performance', icon: ClipboardCheck },
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
    return { range: { from: to - r.ms, to }, rangeLabel: r.label, teams: team ? [team] : [] };
  }, [rangeKey, team]);
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
          <NavLink key={r.to} to={r.to} className={({ isActive }) => `pr-nav-item${isActive ? ' active' : ''}`}>
            <r.icon size={15} /> {r.label}
          </NavLink>
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
        <Outlet context={ctx} />
      </main>
      {adding && <AddDashboardsModal current={dashboards.data ?? []} onClose={() => setAdding(false)} />}
    </div>
  );
}
