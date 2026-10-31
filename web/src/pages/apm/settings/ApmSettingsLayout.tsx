import { useState } from 'react';
import { Link, NavLink, Outlet } from 'react-router-dom';
import { ChevronRight, ExternalLink, Network, PanelLeftClose, PanelLeftOpen, Plus } from 'lucide-react';
import '../../settings/settings.css';
import '../../logs/logs.css';
import '../../../components/shell/configLayout.css';
import './apmSettings.css';

const NAV: { to: string; label: string; isNew?: boolean; end?: boolean }[] = [
  { to: '/apm/settings', label: 'Overview', isNew: true, end: true },
  { to: '/apm/settings/defaults', label: 'Default Settings' },
  { to: '/apm/settings/ingestion', label: 'Ingestion Control' },
  { to: '/apm/settings/pipelines', label: 'Pipelines' },
  { to: '/apm/settings/metrics', label: 'Generate Metrics' },
  { to: '/apm/settings/retention-filters', label: 'Retention Filters' },
];

/** Pages of other products that configure spans too. */
const ELSEWHERE: { to: string; label: string }[] = [
  { to: '/security/appsec/policies/sensitive-data', label: 'Sensitive Data Scanner' },
  { to: '/logs/attributes', label: 'Standard Attributes' },
];

/** APM → Settings: breadcrumb, the settings menu (with the APM setup card) and the selected page. */
export function ApmSettingsLayout() {
  const [collapsed, setCollapsed] = useState(false);
  return (
    <div className="settings-page aps-page">
      <header className="logs-config-header">
        <Link to="/apm" className="logs-config-crumb">
          <Network size={20} /> APM
        </Link>
        <ChevronRight size={18} className="muted" />
        <span>Settings</span>
      </header>
      <div className={`settings-body org-body${collapsed ? ' nav-hidden' : ''}`}>
        {!collapsed && (
          <aside className="settings-nav org-nav logs-config-nav aps-nav" aria-label="APM settings">
            <div className="org-nav-group">
              {NAV.map((i) => (
                <NavLink key={i.to} to={i.to} end={i.end} className={({ isActive }) => `settings-link${isActive ? ' active' : ''}`}>
                  {i.label}
                  {i.isNew && <span className="badge-new">NEW</span>}
                </NavLink>
              ))}
              {ELSEWHERE.map((i) => (
                <Link key={i.to} to={i.to} className="settings-link aps-nav-out">
                  {i.label} <ExternalLink size={14} />
                </Link>
              ))}
            </div>
            <Link to="/apm/instrument" className="aps-setup-card">
              <b>
                APM Setup <ExternalLink size={12} />
              </b>
              <span className="muted">Install a tracing library and manage instrumentation rules</span>
              <Plus size={30} className="aps-setup-plus" />
            </Link>
          </aside>
        )}
        <button className="org-nav-toggle logs-config-toggle" onClick={() => setCollapsed((c) => !c)} title={collapsed ? 'Show navigation' : 'Hide navigation'}>
          {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
        </button>
        <main className="settings-main org-main aps-main">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
