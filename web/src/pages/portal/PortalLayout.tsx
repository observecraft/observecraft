import { NavLink, Outlet } from 'react-router-dom';
import { Code2 } from 'lucide-react';
import './portal.css';

// Catalog is APM's Software Catalog; the portal's own pages are only Scorecards so far.
const TABS = [
  { to: '/apm', label: 'Catalog' },
  { to: '/portal/scorecards', label: 'Scorecards' },
];

/** Developer Portal frame: title and section tabs. */
export function PortalLayout() {
  return (
    <div className="sc-layout">
      <header className="product-header">
        <div className="product-title">
          <Code2 size={20} />
          Developer Portal
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
        </nav>
      </header>
      <div className="sc-section">
        <Outlet />
      </div>
    </div>
  );
}
