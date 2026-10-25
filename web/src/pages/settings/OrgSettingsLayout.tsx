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
}

/** Organization settings navigation. Items without a page yet open a placeholder. */
export const ORG_NAV: { section: string; items: Item[] }[] = [
  {
    section: 'IDENTITY & ACCOUNTS',
    items: [
      { to: 'users', label: 'Users' },
      { to: '/teams', label: 'Teams', chevron: true },
      { to: 'service-accounts', label: 'Service Accounts' },
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
      { to: 'access-tokens', label: 'Access Tokens', badge: 'NEW' },
      { to: 'roles', label: 'Roles' },
      { to: 'client-tokens', label: 'Client Tokens' },
      { to: 'events-api-emails', label: 'Events API Emails' },
      { to: 'cross-org', label: 'Cross-Org Visibility' },
    ],
  },
  {
    section: 'SECURITY',
    items: [
      { to: 'safety-center', label: 'Safety Center', badge: 'NEW' },
      { to: 'public-sharing', label: 'Public Sharing' },
      { to: 'remote-config', label: 'Remote Configuration' },
    ],
  },
  {
    section: 'COMPLIANCE',
    items: [
      { to: 'audit-trail', label: 'Audit Trail' },
      { to: 'sensitive-data-scanner', label: 'Sensitive Data Scanner' },
      { to: 'tagging-policies', label: 'Tagging Policies', badge: 'PREVIEW' },
    ],
  },
  {
    section: 'PRODUCTS',
    items: [
      { to: '/apm', label: 'APM' },
      { to: 'ci', label: 'CI' },
      { to: 'cloud-cost', label: 'Cloud Cost' },
      { to: 'event-management', label: 'Event Management' },
      { to: '/incidents', label: 'Incidents' },
      { to: '/integrations', label: 'Integrations' },
      { to: '/logs', label: 'Logs' },
      { to: 'monitors', label: 'Monitors' },
      { to: 'on-call', label: 'On-Call' },
      { to: 'processes', label: 'Processes' },
      { to: 'rum', label: 'RUM' },
      { to: 'security', label: 'Security' },
      { to: 'software-catalog', label: 'Software Catalog' },
      { to: 'synthetics', label: 'Synthetic Tests' },
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
                  {items.map((i) => (
                    <NavLink key={i.to} to={i.to.startsWith('/') ? i.to : `/organization-settings/${i.to}`} className={({ isActive }) => `settings-link${isActive ? ' active' : ''}`}>
                      {i.label}
                      {i.badge && <span className={i.badge === 'NEW' ? 'badge-new' : 'badge-preview'}>{i.badge}</span>}
                      {i.chevron && <ChevronRight size={16} className="org-nav-chevron" />}
                    </NavLink>
                  ))}
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
