import { useQuery } from '@tanstack/react-query';
import { NavLink, Outlet, useLocation } from 'react-router-dom';
import { profileInsights } from '../../../api/profiles/client';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import './profiles.css';

const TABS = [
  { to: '/apm/profiles/overview', label: 'Overview', isNew: true },
  { to: '/apm/profiles', label: 'Explorer', end: true },
  { to: '/apm/profiles/comparison', label: 'Comparison' },
  { to: '/apm/profiles/insights', label: 'Insights' },
];

/** APM → Profiles: Overview / Explorer / Comparison / Insights, Learn More and the time range. */
export function ProfilesLayout() {
  const { pathname } = useLocation();
  const { range } = useTime();
  const insights = useQuery({ queryKey: ['profiles', 'insights', range], queryFn: () => profileInsights({ range }) });
  const open = insights.data?.items.filter((i) => i.status === 'for_review').length ?? 0;
  // The Overview has no time range, and Comparison picks one per side.
  const timed = !pathname.startsWith('/apm/profiles/overview') && !pathname.startsWith('/apm/profiles/comparison');
  return (
    <div className="pf-layout">
      <div className="pf-subbar">
        <nav className="pf-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `pf-tab${isActive ? ' active' : ''}`}>
              {t.isNew && <span className="pf-new">NEW</span>}
              {t.label}
              {t.label === 'Insights' && open > 0 && <span className="pf-count">{open}</span>}
            </NavLink>
          ))}
        </nav>
        <div className="pf-subbar-right">
          {timed && <TimePicker />}
        </div>
      </div>
      <div className="pf-section">
        <Outlet />
      </div>
    </div>
  );
}
