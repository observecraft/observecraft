import { Link, NavLink, Outlet, useNavigate } from 'react-router-dom';
import { ChevronDown, ExternalLink, Gauge, Plus, Settings } from 'lucide-react';
import { Menu } from '../../components/ui/Dropdown';
import './monitors.css';

const TABS = [
  { to: '/monitors', label: 'List', end: true },
  { to: '/monitors/triggered', label: 'Triggered' },
  { to: '/monitors/downtimes', label: 'Downtimes' },
  { to: '/monitors/quality', label: 'Quality' },
];

/** Monitors product frame: title, section tabs and the create actions. */
export function MonitorsLayout() {
  const navigate = useNavigate();
  return (
    <div className="mon-layout">
      <header className="product-header">
        <div className="product-title">
          <Gauge size={20} />
          Monitors
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
          <Link to="/dashboards?q=monitor" className="product-tab" title="Monitor notification dashboards">
            Analytics <ExternalLink size={14} />
          </Link>
        </nav>
        <div className="mon-header-actions">
          <Link className="link-muted mon-header-link" to="/monitors/settings">
            <Settings size={15} /> Settings
          </Link>
          <Link className="link-muted mon-header-link" to="/monitors/templates">
            Browse Templates
          </Link>
          <div className="mon-split">
            <button className="btn btn-primary" onClick={() => navigate('/monitors/create')}>
              <Plus size={15} /> New Monitor
            </button>
            <Menu
              triggerClassName="btn btn-primary mon-split-caret"
              trigger={<ChevronDown size={15} />}
              items={[
                {
                  label: 'New Metric Monitor',
                  onClick: () => navigate('/monitors/create/metric'),
                },
                {
                  label: 'New APM Monitor',
                  onClick: () => navigate('/monitors/create/apm'),
                },
                {
                  label: 'Start from a template',
                  onClick: () => navigate('/monitors/templates'),
                },
                {
                  label: 'Import from JSON',
                  onClick: () => navigate('/monitors/create?import=1'),
                },
              ]}
            />
          </div>
        </div>
      </header>
      <div className="mon-section">
        <Outlet />
      </div>
    </div>
  );
}
