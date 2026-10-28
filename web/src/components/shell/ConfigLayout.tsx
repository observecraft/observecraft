import { useState, type ReactNode } from 'react';
import { Link, NavLink, Outlet } from 'react-router-dom';
import { ChevronRight, PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import '../../pages/settings/settings.css';
import '../../pages/logs/logs.css';
import './configLayout.css';

export interface ConfigNavItem {
  to: string;
  label: string;
  /** Shown indented under the item before it. */
  nested?: boolean;
  isNew?: boolean;
  /** A label over the nested items after it, not a page. */
  heading?: boolean;
}

export interface ConfigNavGroup {
  section: string;
  items: ConfigNavItem[];
}

/**
 * A product's configuration frame: breadcrumb (`Root › Title`), a collapsible section menu and the
 * selected page (the layout of Logs › Configuration, shared by other products' settings).
 */
export function ConfigLayout({ root, title, nav, label }: { root: { to: string; label: string; icon: ReactNode }; title: string; nav: ConfigNavGroup[]; label: string }) {
  const [collapsed, setCollapsed] = useState(false);
  return (
    <div className="settings-page">
      <header className="logs-config-header">
        <Link to={root.to} className="logs-config-crumb">
          {root.icon} {root.label}
        </Link>
        <ChevronRight size={18} className="muted" />
        <span>{title}</span>
      </header>
      <div className={`settings-body org-body${collapsed ? ' nav-hidden' : ''}`}>
        {!collapsed && (
          <aside className="settings-nav org-nav logs-config-nav" aria-label={label}>
            {nav.map((g) => (
              <div key={g.section} className="org-nav-group">
                <div className="org-nav-section">{g.section}</div>
                {g.items.map((i) =>
                  i.heading ? (
                    <span key={i.label} className="settings-link config-link-heading">
                      {i.label}
                    </span>
                  ) : (
                    <NavLink key={i.to} to={i.to} className={({ isActive }) => `settings-link${i.nested ? ' config-link-nested' : ''}${isActive ? ' active' : ''}`}>
                      {i.label}
                      {i.isNew && <span className="badge-new">NEW</span>}
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

/** A settings page whose feature isn't available yet. */
export function ConfigNotAvailable({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="settings-pad">
      <h1 className="settings-title">{title}</h1>
      <section className="settings-card settings-empty">
        <p className="muted">{children ?? `${title} isn't available yet.`}</p>
      </section>
    </div>
  );
}
