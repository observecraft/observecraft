import { useState } from 'react';
import { NavLink, Outlet, useLocation } from 'react-router-dom';
import { ChevronRight, PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react';
import { SettingsHeader } from './SettingsHeader';
import './settings.css';

interface Item {
  to: string;
  label: string;
  badge?: 'NEW' | 'PREVIEW';
  chevron?: boolean;
  /** No page yet: listed (greyed) so the roadmap stays visible, but not a link. */
  soon?: boolean;
}

/** Organization settings navigation, Datadog's sections; items without a page yet are marked `soon`. */
export const ORG_NAV: { section: string; items: Item[] }[] = [
  {
    section: 'IDENTITY & ACCOUNTS',
    items: [
      { to: 'users', label: 'Users' },
      { to: '/teams', label: 'Teams', chevron: true },
      { to: 'service-accounts', label: 'Service Accounts', soon: true },
    ],
  },
  {
    section: 'AUTHENTICATION',
    items: [
      { to: 'login-methods', label: 'Login Methods' },
      { to: 'saml-mappings', label: 'SAML Group Mappings' },
    ],
  },
  {
    section: 'ACCESS',
    items: [
      { to: 'api-keys', label: 'API Keys' },
      { to: 'access-tokens', label: 'Access Tokens', badge: 'NEW', soon: true },
      { to: 'roles', label: 'Roles' },
      { to: 'client-tokens', label: 'Client Tokens', soon: true },
      { to: 'events-api-emails', label: 'Events API Emails', soon: true },
      { to: 'cross-org', label: 'Cross-Org Visibility', soon: true },
    ],
  },
  {
    section: 'SECURITY',
    items: [
      { to: 'safety-center', label: 'Safety Center', badge: 'NEW', soon: true },
      { to: 'public-sharing', label: 'Public Sharing', soon: true },
      { to: 'remote-config', label: 'Remote Configuration', soon: true },
    ],
  },
  {
    section: 'COMPLIANCE',
    items: [
      { to: 'audit-trail', label: 'Audit Trail' },
      { to: 'data-retention', label: 'Data Retention' },
      { to: 'sensitive-data-scanner', label: 'Sensitive Data Scanner', soon: true },
      { to: 'tagging-policies', label: 'Tagging Policies', badge: 'PREVIEW', soon: true },
    ],
  },
  {
    section: 'PRODUCTS',
    items: [
      { to: '/apm', label: 'APM' },
      { to: 'ci', label: 'CI', soon: true },
      { to: 'cloud-cost', label: 'Cloud Cost', soon: true },
      { to: 'event-management', label: 'Event Management', soon: true },
      { to: '/incidents', label: 'Incidents' },
      { to: '/integrations', label: 'Integrations' },
      { to: '/logs', label: 'Logs' },
      { to: 'monitors', label: 'Monitors', soon: true },
      { to: 'on-call', label: 'On-Call', soon: true },
      { to: 'processes', label: 'Processes', soon: true },
      { to: 'rum', label: 'RUM', soon: true },
      { to: 'security', label: 'Security', soon: true },
      { to: 'software-catalog', label: 'Software Catalog', soon: true },
      { to: 'synthetics', label: 'Synthetic Tests', soon: true },
    ],
  },
];

export function OrgSettingsLayout() {
  const [filter, setFilter] = useState('');
  const [collapsed, setCollapsed] = useState(false);
  const f = filter.trim().toLowerCase();
  return (
    <div className="settings-page">
      <SettingsHeader kind="organization" />
      <div className={`settings-body org-body${collapsed ? ' nav-hidden' : ''}`}>
        {!collapsed && (
          <aside className="settings-nav org-nav">
            <label className="settings-search">
              <Search size={16} />
              <input placeholder="Search" value={filter} onChange={(e) => setFilter(e.target.value)} />
            </label>
            {ORG_NAV.map((g) => {
              const items = g.items.filter((i) => !f || i.label.toLowerCase().includes(f));
              if (items.length === 0) return null;
              return (
                <div key={g.section} className="org-nav-group">
                  <div className="org-nav-section">{g.section}</div>
                  {items.map((i) =>
                    i.soon ? (
                      <span key={i.to} className="settings-link settings-link-soon" title={`${i.label} isn't built yet`}>
                        {i.label}
                        <span className="badge-soon">SOON</span>
                      </span>
                    ) : (
                      <NavLink key={i.to} to={i.to.startsWith('/') ? i.to : `/organization-settings/${i.to}`} className={({ isActive }) => `settings-link${isActive ? ' active' : ''}`}>
                        {i.label}
                        {i.badge && <span className={i.badge === 'NEW' ? 'badge-new' : 'badge-preview'}>{i.badge}</span>}
                        {i.chevron && <ChevronRight size={16} className="org-nav-chevron" />}
                      </NavLink>
                    ),
                  )}
                </div>
              );
            })}
          </aside>
        )}
        <button className="org-nav-toggle" onClick={() => setCollapsed((c) => !c)} title={collapsed ? 'Show navigation' : 'Hide navigation'}>
          {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
        </button>
        <main className="settings-main org-main">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

export function OrgComingSoon() {
  const { pathname } = useLocation();
  const slug = pathname.split('/').pop() ?? '';
  const label = ORG_NAV.flatMap((g) => g.items).find((i) => i.to === slug)?.label ?? 'This page';
  return (
    <div className="settings-pad">
      <h1 className="settings-title">{label}</h1>
      <section className="settings-card settings-empty">
        <p className="muted">{label} hasn't been built yet.</p>
      </section>
    </div>
  );
}
