import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { ChevronDown, Plus, ServerCog } from 'lucide-react';
import { TimePicker } from '../../components/shell/PageHeader';
import { Menu } from '../../components/ui/Dropdown';
import './fleet.css';

const TABS = [
  { to: '/fleet', label: 'Fleet View', end: true },
  { to: '/fleet/configuration', label: 'Configuration', isNew: true },
  { to: '/fleet/upgrades', label: 'Upgrades' },
];

/** Integrations → Fleet Automation frame: title menu, the three views, Install Agents and the time range. */
export function FleetLayout() {
  const navigate = useNavigate();
  return (
    <div className="fl-layout">
      <header className="product-header">
        <div className="product-title">
          <ServerCog size={20} />
          <Menu
            align="left"
            triggerClassName="fl-title-menu"
            trigger={
              <>
                Fleet Automation <ChevronDown size={16} />
              </>
            }
            items={[...TABS.map((t) => ({ label: t.label, onClick: () => navigate(t.to) })), { label: 'Install Agents', onClick: () => navigate('/infrastructure/add') }]}
          />
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.isNew && <span className="product-tab-new badge-new">NEW</span>}
              {t.label}
            </NavLink>
          ))}
        </nav>
        <div className="product-config fl-header-actions">
          <button className="btn btn-primary" onClick={() => navigate('/infrastructure/add')}>
            <Plus size={14} /> Install Agents
          </button>
          <TimePicker compact />
        </div>
      </header>
      <div className="fl-body">
        <Outlet />
      </div>
    </div>
  );
}
