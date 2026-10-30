import { NavLink, Outlet, useLocation } from 'react-router-dom';
import { Network, Settings } from 'lucide-react';
import { TimeProvider } from '../../components/shell/TimeContext';
import './apm.css';

const TABS = [
  { to: '/apm', label: 'Service', match: (p: string) => p === '/apm' || p.startsWith('/apm/services') },
  { to: '/apm/traces', label: 'Traces', match: (p: string) => p.startsWith('/apm/traces') },
  { to: '/apm/profiles', label: 'Profiles', match: (p: string) => p.startsWith('/apm/profiles') },
  { to: '/apm/debugger', label: 'Debugger', match: (p: string) => p.startsWith('/apm/debugger') },
];

/** APM product frame: its own time range (past hour by default) and product tabs. */
export function ApmLayout() {
  const { pathname } = useLocation();
  return (
    <TimeProvider initial="1h">
      <div className="apm-layout">
        {/* A service page has its own breadcrumb header in place of the product tabs. */}
        {!pathname.startsWith('/apm/services/') && (
          <header className="product-header">
            <div className="product-title">
              <Network size={20} />
              APM
            </div>
            <nav className="product-tabs">
              {TABS.map((t) => (
                <NavLink key={t.to} to={t.to} end className={() => `product-tab${t.match(pathname) ? ' active' : ''}`}>
                  {t.label}
                </NavLink>
              ))}
            </nav>
            <NavLink to="/apm/settings" className="btn product-config">
              <Settings size={14} /> Settings
            </NavLink>
          </header>
        )}
        <div className="apm-section">
          <Outlet />
        </div>
      </div>
    </TimeProvider>
  );
}

export function ApmComingSoon({ title }: { title: string }) {
  return <div className="placeholder-page">{title} hasn't been built yet.</div>;
}
