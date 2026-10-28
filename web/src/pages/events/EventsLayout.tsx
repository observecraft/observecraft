import { Link, NavLink, Outlet, useLocation } from 'react-router-dom';
import { ListChecks, Plus, Settings } from 'lucide-react';
import './events.css';

const TABS = [
  { to: '/event/inbox', label: 'Triage Inbox' },
  { to: '/event/explorer', label: 'All Events' },
  { to: '/event/summary', label: 'Summary' },
];

/** Event Management frame: product title, Triage Inbox / All Events / Summary, Settings. */
export function EventsLayout() {
  const { pathname } = useLocation();
  return (
    <div className="events-layout">
      <header className="product-header">
        <div className="product-title">
          <ListChecks size={20} />
          Event Management
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
        </nav>
        <div className="ev-header-right">
          <Link className="btn" to="/event/settings" title="Event Management settings">
            <Settings size={14} /> Settings
          </Link>
          {pathname.startsWith('/event/inbox') && (
            <Link className="btn btn-primary" to="/event/settings/correlators?new=1" title="Correlators group related events into work items">
              <Plus size={14} /> New Correlator
            </Link>
          )}
        </div>
      </header>
      <div className="events-section">
        <Outlet />
      </div>
    </div>
  );
}

export function EventsSummary() {
  return <div className="placeholder-page">Summary hasn't been designed yet.</div>;
}
