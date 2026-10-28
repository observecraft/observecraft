import { NavLink, Outlet } from 'react-router-dom';
import { LayoutList } from 'lucide-react';
import './infrastructure.css';

const TABS = [
  { to: '/infrastructure/catalog', label: 'Catalog' },
  { to: '/cloudcraft', label: 'Cloudcraft' },
  { to: '/infrastructure/changes', label: 'Changes', preview: true },
  { to: '/infrastructure/policies', label: 'Policies', preview: true },
];

/** Infrastructure → Resources frame: the Catalog, Cloudcraft, Changes and Policies tabs. */
export function ResourcesLayout() {
  return (
    <div className="rc-layout">
      <header className="product-header">
        <div className="product-title">
          <LayoutList size={20} /> Resources
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
              {t.preview && <span className="rc-preview">PREVIEW</span>}
            </NavLink>
          ))}
        </nav>
      </header>
      <div className="rc-section">
        <Outlet />
      </div>
    </div>
  );
}
