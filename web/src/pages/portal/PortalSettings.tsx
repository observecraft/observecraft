import { Link, NavLink, Outlet, useParams } from 'react-router-dom';
import { ChevronRight, Code2 } from 'lucide-react';
import '../integrations/integrations.css';
import './portal.css';
import './home.css';
import './settings.css';

const GROUPS: { title: string; items: { to: string; label: string; soon?: boolean }[] }[] = [
  {
    title: 'Build your software catalog',
    items: [
      { to: 'sources', label: 'Entities Sources' },
      { to: 'entity-types', label: 'Entity Types', soon: true },
    ],
  },
  {
    title: 'Edit your catalog',
    items: [
      { to: 'service-remapping', label: 'Service Remapping', soon: true },
      { to: 'service-overrides', label: 'Manage Service Overrides', soon: true },
      { to: 'excluded-repos', label: 'Excluded Repos', soon: true },
    ],
  },
  { title: 'Integrations', items: [{ to: 'configurations', label: 'Configurations' }] },
];

/** Developer Portal → Manage: catalog sources and the source connectors. */
export function PortalSettings() {
  return (
    <div className="sc-layout">
      <header className="product-header">
        <div className="product-title ps-breadcrumb">
          <Code2 size={20} />
          <Link to="/portal/home">Developer Portal</Link>
          <ChevronRight size={16} className="ps-crumb-sep" />
          <span>Manage</span>
        </div>
      </header>
      <div className="ps-body">
        <nav className="ps-nav" aria-label="Developer Portal settings">
          {GROUPS.map((g) => (
            <div key={g.title} className="ps-nav-group">
              <div className="ps-nav-title">{g.title}</div>
              {g.items.map((i) => (
                <NavLink key={i.to} to={`/portal/settings/${i.to}`} className={({ isActive }) => `ps-nav-item${isActive ? ' active' : ''}`}>
                  <span>{i.label}</span>
                  {i.soon && <span className="ps-soon">soon</span>}
                </NavLink>
              ))}
            </div>
          ))}
        </nav>
        <main className="ps-main">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

const SOON_TITLES: Record<string, string> = {
  'entity-types': 'Entity Types',
  'service-remapping': 'Service Remapping',
  'service-overrides': 'Manage Service Overrides',
  'excluded-repos': 'Excluded Repos',
};

/** Settings pages not built yet. */
export function PortalSettingsSoon() {
  const { page = '' } = useParams();
  return (
    <div className="ps-page">
      <h1 className="ps-title">{SOON_TITLES[page] ?? 'Settings'}</h1>
      <div className="ph-card ps-soon-card">
        <strong>Coming soon</strong>
        <p className="sc-muted">This part of the Developer Portal isn't available yet.</p>
      </div>
    </div>
  );
}
