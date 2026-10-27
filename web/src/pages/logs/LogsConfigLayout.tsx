import { useState } from 'react';
import { Link, NavLink, Outlet, useLocation } from 'react-router-dom';
import { ChevronRight, ExternalLink, PanelLeftClose, PanelLeftOpen, ScrollText } from 'lucide-react';
import '../settings/settings.css';
import './logs.css';

interface Item {
  slug: string;
  label: string;
  /** Lives elsewhere in the app; shown with an external-link mark. */
  href?: string;
  /** No page yet: listed (greyed), not a link. */
  soon?: boolean;
}

/** Log Configuration navigation. Each slug is served at /logs/<slug>. */
export const LOGS_CONFIG_NAV: { section: string; isNew?: boolean; items: Item[] }[] = [
  {
    section: 'PROCESSING',
    items: [
      { slug: 'pipelines', label: 'Pipelines', soon: true },
      { slug: 'attributes', label: 'Standard Attributes', soon: true },
      { slug: 'sensitive-data-scanner', label: 'Sensitive Data Scanner', href: '/organization-settings/sensitive-data-scanner', soon: true },
    ],
  },
  {
    section: 'ROUTING',
    items: [
      { slug: 'indexes', label: 'Indexes' },
      { slug: 'flex-controls', label: 'Flex Logs Controls', soon: true },
      { slug: 'archives', label: 'Archiving & Forwarding' },
      { slug: 'rehydrations', label: 'Rehydrations' },
    ],
  },
  { section: 'STREAM INSIGHTS', items: [{ slug: 'generate-metrics', label: 'Generate Metrics', soon: true }] },
  { section: 'ACCESS CONTROL', items: [{ slug: 'data-access', label: 'Data Access', soon: true }] },
  {
    section: 'MIGRATION',
    isNew: true,
    items: [
      { slug: 'migration/import', label: 'Import', soon: true },
      { slug: 'migration/translate', label: 'Translate', soon: true },
    ],
  },
];

export const LOGS_CONFIG_SLUGS = LOGS_CONFIG_NAV.flatMap((g) => g.items.filter((i) => !i.href).map((i) => i.slug));

/** Logs › Configuration: a collapsible section menu beside the selected configuration page. */
export function LogsConfigLayout() {
  const [collapsed, setCollapsed] = useState(false);
  return (
    <div className="settings-page">
      <header className="logs-config-header">
        <Link to="/logs" className="logs-config-crumb">
          <ScrollText size={22} /> Logs
        </Link>
        <ChevronRight size={18} className="muted" />
        <span>Configuration</span>
      </header>
      <div className={`settings-body org-body${collapsed ? ' nav-hidden' : ''}`}>
        {!collapsed && (
          <aside className="settings-nav org-nav logs-config-nav" aria-label="Log configuration">
            {LOGS_CONFIG_NAV.map((g) => (
              <div key={g.section} className="org-nav-group">
                <div className="org-nav-section">
                  {g.section} {g.isNew && <span className="badge-new">NEW</span>}
                </div>
                {g.items.map((i) =>
                  i.soon ? (
                    <span key={i.slug} className="settings-link settings-link-soon" title={`${i.label} isn't built yet`}>
                      {i.label}
                      <span className="badge-soon">SOON</span>
                    </span>
                  ) : (
                    <NavLink key={i.slug} to={i.href ?? `/logs/${i.slug}`} className={({ isActive }) => `settings-link${isActive ? ' active' : ''}`}>
                      {i.label}
                      {i.href && <ExternalLink size={16} className="org-nav-chevron" />}
                    </NavLink>
                  ),
                )}
              </div>
            ))}
          </aside>
        )}
        <button className="org-nav-toggle logs-config-toggle" onClick={() => setCollapsed((c) => !c)} title={collapsed ? 'Show navigation' : 'Hide navigation'}>
          {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
        </button>
        <main className="settings-main org-main">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

export function LogsConfigComingSoon() {
  const { pathname } = useLocation();
  const slug = pathname.replace(/^\/logs\//, '');
  const label = LOGS_CONFIG_NAV.flatMap((g) => g.items).find((i) => i.slug === slug)?.label ?? 'This page';
  return (
    <div className="settings-pad">
      <h1 className="settings-title">{label}</h1>
      <section className="settings-card settings-empty">
        <p className="muted">{label} hasn't been built yet.</p>
      </section>
    </div>
  );
}
