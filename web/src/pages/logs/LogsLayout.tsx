import { Link, NavLink, Outlet } from 'react-router-dom';
import { ScrollText } from 'lucide-react';
import './logs.css';

const TABS = [
  { to: '/logs', label: 'Log Explorer', end: true },
  { to: '/logs/livetail', label: 'Live Tail' },
  { to: '/notebooks', label: 'Notebooks' },
  { to: '/logs/archive-search', label: 'Archive Search', isNew: true },
];

/** Logs product frame: product title + section tabs, with the active section below. */
export function LogsLayout() {
  return (
    <div className="logs-layout">
      <header className="product-header">
        <div className="product-title">
          <ScrollText size={20} />
          Logs
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.isNew && <span className="product-tab-new badge-new">NEW</span>}
              {t.label}
            </NavLink>
          ))}
        </nav>
        <Link to="/logs/pipelines" className="btn product-config">
          Log Configuration
        </Link>
      </header>
      <div className="logs-section">
        <Outlet />
      </div>
    </div>
  );
}
