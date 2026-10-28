import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { Code2, Settings } from 'lucide-react';
import './portal.css';

const TABS = [
  { to: '/portal/home', label: 'Home' },
  { to: '/portal/catalog', label: 'Catalog' },
  { to: '/portal/scorecards', label: 'Scorecards' },
  { to: '/portal/reports', label: 'Reports' },
  { to: '/portal/surveys', label: 'Surveys' },
  { to: '/portal/actions', label: 'Actions' },
];

/** Developer Portal frame: title, section tabs and the portal's settings. */
export function PortalLayout() {
  const navigate = useNavigate();
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
        <div className="product-config">
          <button className="btn" onClick={() => navigate('/portal/settings')}>
            <Settings size={14} /> Settings
          </button>
        </div>
      </header>
      <div className="sc-section">
        <Outlet />
      </div>
    </div>
  );
}
