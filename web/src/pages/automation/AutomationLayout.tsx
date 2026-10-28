import { Link, NavLink, Outlet, useLocation } from 'react-router-dom';
import { ChevronDown, Zap } from 'lucide-react';
import { usePopover } from '../../components/ui/Dropdown';
import './automation.css';

type Product = 'workflows' | 'apps' | 'datastores' | 'actions';

const SUB_TABS: Partial<Record<Product, { to: string; label: string; end?: boolean }[]>> = {
  workflows: [
    { to: '/automation/intro', label: 'Intro' },
    { to: '/automation', label: 'All Workflows', end: true },
    { to: '/automation/blueprints', label: 'Blueprints' },
    { to: '/automation/analytics', label: 'Analytics' },
  ],
  apps: [
    { to: '/automation/apps/intro', label: 'Intro' },
    { to: '/automation/apps', label: 'All Apps', end: true },
    { to: '/automation/apps/blueprints', label: 'Blueprints' },
  ],
};

export function productOf(pathname: string): Product {
  if (pathname.startsWith('/automation/apps')) return 'apps';
  if (pathname.startsWith('/automation/actions')) return 'actions';
  if (pathname.startsWith('/automation/datastores')) return 'datastores';
  return 'workflows';
}

function ProductTab({ to, label, active, menu }: { to: string; label: string; active: boolean; menu?: { to: string; label: string }[] }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className={`product-tab au-tab${active ? ' active' : ''}`} ref={ref}>
      <Link to={to} className="au-tab-link">
        {label}
      </Link>
      {menu && (
        <button className="au-caret" aria-label={`${label} menu`} aria-expanded={open} onClick={() => setOpen((o) => !o)}>
          <ChevronDown size={14} />
        </button>
      )}
      {open && menu && (
        <div className="au-menu" role="menu">
          {menu.map((m) => (
            <Link key={m.to} to={m.to} role="menuitem" className="au-menu-item" onClick={() => setOpen(false)}>
              {m.label}
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}

/** Automation frame: product header (Workflow Automation, Apps, Datastores, Action Catalog) and sub-tabs. */
export function AutomationLayout() {
  const { pathname } = useLocation();
  const product = productOf(pathname);
  const subs = SUB_TABS[product];
  return (
    <div className="au-layout">
      <header className="product-header">
        <div className="product-title">
          <Zap size={20} />
          Automation
        </div>
        <nav className="product-tabs">
          <ProductTab
            to="/automation"
            label="Workflow Automation"
            active={product === 'workflows'}
            menu={[
              { to: '/automation/intro', label: 'Intro' },
              { to: '/automation', label: 'All Workflows' },
              { to: '/automation/blueprints', label: 'Blueprints' },
            ]}
          />
          <ProductTab
            to="/automation/apps"
            label="Apps"
            active={product === 'apps'}
            menu={[
              { to: '/automation/apps/intro', label: 'Intro' },
              { to: '/automation/apps', label: 'All Apps' },
              { to: '/automation/apps/blueprints', label: 'Blueprints' },
            ]}
          />
          <ProductTab to="/automation/datastores" label="Datastores" active={product === 'datastores'} />
          <ProductTab to="/automation/actions" label="Action Catalog" active={product === 'actions'} />
        </nav>
      </header>
      {subs && (
        <nav className="au-subtabs">
          {subs.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `au-subtab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
        </nav>
      )}
      <div className="au-section">
        <Outlet />
      </div>
    </div>
  );
}
