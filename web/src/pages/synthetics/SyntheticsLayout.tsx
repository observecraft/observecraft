import type { ReactNode } from 'react';
import { Link, NavLink, Outlet, useNavigate } from 'react-router-dom';
import { ChevronDown, Plus, Radar, Settings } from 'lucide-react';
import { Menu } from '../../components/ui/Dropdown';
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
        </nav>
        <div className="mon-header-actions">
          <Link className="link-muted mon-header-link" to="/synthetics/settings/default">
            <Settings size={15} /> Settings
          </Link>
          <div className="mon-split">
            <button className="btn btn-primary" onClick={() => navigate('/synthetics/create')}>
              <Plus size={15} /> New Test
            </button>
            <Menu
              triggerClassName="btn btn-primary mon-split-caret"
              trigger={<ChevronDown size={15} aria-label="More create options" />}
              items={[
                { label: 'New API Test', onClick: () => navigate('/synthetics/create') },
                { label: 'New Suite', onClick: () => navigate('/synthetics/suites/new') },
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
