import { useState, type ReactNode } from 'react';
import { Link, NavLink, Outlet, useNavigate } from 'react-router-dom';
import { ChevronDown, Plus, Radar, Settings } from 'lucide-react';
import { Menu } from '../../components/ui/Dropdown';
import { useDashboardSearch } from '../dashboards/queries';
import { NewTestModal } from './NewTest';
import '../monitors/monitors.css';
import './synthetics.css';
import './syntheticsProduct.css';

const TABS = [
  { to: '/synthetics', label: 'Overview', end: true },
  { to: '/synthetics/suites', label: 'Test Suites' },
  { to: '/synthetics/tests', label: 'Tests' },
  { to: '/synthetics/explorer', label: 'Results Explorer' },
  { to: '/synthetics/test-coverage', label: 'Test Coverage' },
];

/** Synthetic Monitoring & Testing frame: product switcher, tabs, settings and the New Test split button. */
export function SyntheticsLayout() {
  const navigate = useNavigate();
  const [creating, setCreating] = useState(false);
  // The Dashboards menu: dashboards about synthetics (the built-in one and the org's own).
  const dashboards = useDashboardSearch({ query: 'synthetics', preset: 'all', listId: null, team: null, sort: { field: 'title', dir: 'asc' }, offset: 0, limit: 10 });
  return (
    <div className="mon-layout">
      <header className="product-header syn-product-header">
        <Menu
          align="left"
          triggerClassName="product-title syn-product-switch"
          trigger={
            <>
              <Radar size={20} /> Synthetic Monitoring &amp; Testing <ChevronDown size={16} />
            </>
          }
          items={[
            { label: 'Synthetic Monitoring & Testing', onClick: () => navigate('/synthetics') },
            { label: 'Journey Monitoring', onClick: () => navigate('/journeys') },
            { label: 'Real User Monitoring', onClick: () => navigate('/rum') },
            { label: 'Product Analytics', onClick: () => navigate('/product-analytics') },
          ]}
        />
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
          <Menu
            triggerClassName="product-tab syn-tab-menu"
            trigger={
              <>
                Dashboards <ChevronDown size={15} />
              </>
            }
            items={[
              ...(dashboards.data?.dashboards ?? []).map((d) => ({ label: d.title, onClick: () => navigate(`/dashboards/${d.id}`) })),
              { label: 'All synthetics dashboards', onClick: () => navigate('/dashboards?q=synthetics') },
            ]}
          />
        </nav>
        <div className="mon-header-actions">
          <Link className="link-muted mon-header-link" to="/synthetics/settings/default">
            <Settings size={15} /> Settings
          </Link>
          <div className="mon-split">
            <button className="btn btn-primary" onClick={() => setCreating(true)}>
              <Plus size={15} /> New Test
            </button>
            <Menu
              triggerClassName="btn btn-primary mon-split-caret"
              trigger={<ChevronDown size={15} aria-label="More create options" />}
              items={[
                { label: 'New API Test', onClick: () => navigate('/synthetics/templates') },
                { label: 'New Suite', onClick: () => navigate('/synthetics/suites/new') },
              ]}
            />
          </div>
        </div>
      </header>
      <div className="mon-section">
        <Outlet />
      </div>
      {creating && <NewTestModal onClose={() => setCreating(false)} />}
    </div>
  );
}

/** A Digital Experience page that isn't available yet, inside the synthetics frame. */
export function SyntheticsNotAvailable({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="syn-page">
      <section className="syn-unavailable">
        <Radar size={34} strokeWidth={1.4} />
        <h2>{title}</h2>
        <p className="muted">{children}</p>
      </section>
    </div>
  );
}
