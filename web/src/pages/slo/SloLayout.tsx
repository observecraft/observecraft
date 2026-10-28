import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { Plus, Target } from 'lucide-react';
import '../monitors/monitors.css';
import './slo.css';

const TABS = [
  { to: '/slo', label: 'List', end: true },
  { to: '/slo/corrections', label: 'Corrections' },
];

/** Service Level Objectives frame: title, section tabs and the create action. */
export function SloLayout() {
  const navigate = useNavigate();
  return (
    <div className="mon-layout">
      <header className="product-header">
        <div className="product-title">
          <Target size={20} />
          SLOs
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
        </nav>
        <div className="mon-header-actions">
          <button className="btn btn-primary" onClick={() => navigate('/slo/create')}>
            <Plus size={15} /> New SLO
          </button>
        </div>
      </header>
      <div className="mon-section">
        <Outlet />
      </div>
    </div>
  );
}
