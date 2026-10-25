import { NavLink, Outlet } from 'react-router-dom';
import './integrations.css';

const TABS = [
  { to: '/integrations', label: 'Integration Catalog', end: true },
  { to: '/integrations/marketplace', label: 'Marketplace' },
  { to: '/integrations/reference-tables', label: 'Reference Tables' },
  { to: '/integrations/source-code', label: 'Link Source Code' },
];

export function IntegrationsLayout() {
  return (
    <div className="int-layout">
      <header className="product-header int-header">
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
        </nav>
      </header>
      <div className="int-section">
        <Outlet />
      </div>
    </div>
  );
}

export function IntegrationsComingSoon({ title }: { title: string }) {
  return <div className="placeholder-page">{title} hasn't been designed yet.</div>;
}
