import { NavLink, Outlet } from 'react-router-dom';
import { Binoculars } from 'lucide-react';
import './watchdog.css';

const TABS = [
  { to: '/watchdog', label: 'Alerts', end: true },
  { to: '/watchdog/external-provider-status', label: 'External Provider Status' },
];

/** Watchdog product frame: title and its two views. */
export function WatchdogLayout() {
  return (
    <div className="wd-layout">
      <header className="product-header">
        <div className="product-title">
          <Binoculars size={20} />
          Watchdog
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
        </nav>
      </header>
      <Outlet />
    </div>
  );
}

/** External Provider Status: third-party provider outages. No source for them exists yet. */
export function ExternalProviderStatus() {
  return (
    <div className="wd-page">
      <div className="wd-empty">
        <span className="wd-empty-icon">
          <Binoculars size={56} strokeWidth={1.2} />
        </span>
        <h2>External provider status isn't available yet</h2>
        <p>
          ObserveCraft doesn't receive status feeds from cloud and SaaS providers yet. Until it does, outages on their side show up through your own telemetry: error rate and latency increases on the
          services that call them, in Watchdog's alerts.
        </p>
      </div>
    </div>
  );
}
