import { Link, NavLink, Outlet, useNavigate } from 'react-router-dom';
import { ChevronDown, Plus, Settings, Workflow } from 'lucide-react';
import { TimeProvider } from '../../components/shell/TimeContext';
import { Menu } from '../../components/ui/Dropdown';
import './delivery.css';

const CI_TABS = [
  { to: '/ci', label: 'Overview', end: true },
  { to: '/ci/explorer', label: 'Explorer' },
  { to: '/ci/flaky', label: 'Flaky Management' },
];

/** CI/CD Optimization frame: product switcher, tabs, settings and Add Repository. */
export function CiLayout() {
  const navigate = useNavigate();
  return (
    <TimeProvider initial="1w">
      <div className="ci-layout">
        <header className="product-header">
          <Menu
            align="left"
            triggerClassName="product-title ci-product-switch"
            trigger={
              <>
                <Workflow size={20} /> CI/CD Optimization <span className="badge-new">NEW</span> <ChevronDown size={16} />
              </>
            }
            items={[
              { label: 'CI/CD Optimization', onClick: () => navigate('/ci') },
              {
                label: 'Delivery Performance (DORA)',
                onClick: () => navigate('/delivery/dora'),
              },
            ]}
          />
          <nav className="product-tabs">
            {CI_TABS.map((t) => (
              <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
                {t.label}
              </NavLink>
            ))}
          </nav>
          <div className="ci-header-actions">
            <Link className="btn" to="/ci/settings">
              <Settings size={14} /> Settings
            </Link>
            <Link className="btn btn-primary" to="/ci/settings#add-repository">
              <Plus size={14} /> Add Repository
            </Link>
          </div>
        </header>
        <div className="ci-section">
          <Outlet />
        </div>
      </div>
    </TimeProvider>
  );
}

/** Delivery Performance frame (DORA metrics). */
export function DeliveryLayout() {
  return (
    <div className="ci-layout">
      <header className="product-header">
        <div className="product-title">
          <Workflow size={20} /> Delivery Performance
        </div>
        <nav className="product-tabs">
          <NavLink to="/delivery/dora" className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            DORA Metrics
          </NavLink>
          <NavLink to="/delivery/settings" className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Settings
          </NavLink>
        </nav>
      </header>
      <div className="ci-section">
        <Outlet />
      </div>
    </div>
  );
}
