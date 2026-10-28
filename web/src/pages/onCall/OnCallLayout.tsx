import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { PhoneCall, Settings, UserRound } from 'lucide-react';
import '../monitors/monitors.css';
import './onCall.css';

const TABS = [
  { to: '/on-call', label: 'Summary', end: true },
  { to: '/on-call/teams', label: 'Teams' },
  { to: '/on-call/pages', label: 'Pages' },
  { to: '/on-call/schedules', label: 'Schedules' },
  { to: '/on-call/escalation-policies', label: 'Escalation Policies' },
  { to: '/on-call/analytics', label: 'Analytics' },
];

/** Incident Response → On-Call frame: title, tabs, the member's profile and settings. */
export function OnCallLayout() {
  const navigate = useNavigate();
  return (
    <div className="mon-layout">
      <header className="product-header">
        <div className="product-title">
          <PhoneCall size={20} />
          On-Call
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
        </nav>
        <div className="mon-header-actions">
          <NavLink to="/on-call/profile" className={({ isActive }) => `onc-header-link${isActive ? ' active' : ''}`}>
            <UserRound size={15} /> My On-Call Profile
          </NavLink>
          <button className="btn" onClick={() => navigate('/monitors/settings')} title="Notification channels used to reach responders">
            <Settings size={14} /> Settings
          </button>
        </div>
      </header>
      <div className="mon-section onc-section">
        <Outlet />
      </div>
    </div>
  );
}
