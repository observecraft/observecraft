import { Link, NavLink, Outlet } from 'react-router-dom';
import { Gauge, Settings } from 'lucide-react';
import { TimeProvider } from '../../components/shell/TimeContext';
import './metrics.css';

const tab = ({ isActive }: { isActive: boolean }) => `product-tab${isActive ? ' active' : ''}`;

/** Metrics product frame: Overview, Explorer, Catalog and Derived Metrics share one time range (past hour by default). */
export function MetricsLayout() {
  return (
    <TimeProvider initial="1h">
      <div className="metrics-layout">
        <header className="product-header">
          <div className="product-title">
            <Gauge size={20} />
            Metrics
          </div>
          <nav className="product-tabs">
            <NavLink to="/metrics/overview" className={tab}>
              Overview
            </NavLink>
            <NavLink to="/metrics" end className={tab}>
              Explorer
            </NavLink>
            <NavLink to="/metrics/catalog" className={tab}>
              Catalog
            </NavLink>
            <NavLink to="/metrics/derived" className={tab}>
              Derived Metrics <span className="badge-new mx-tab-new">NEW</span>
            </NavLink>
          </nav>
          <Link to="/metrics/settings" className="mx-settings-link">
            <Settings size={15} /> Settings
          </Link>
        </header>
        <div className="metrics-section">
          <Outlet />
        </div>
      </div>
    </TimeProvider>
  );
}

export const KIND_LABEL: Record<string, string> = {
  gauge: 'Gauge',
  sum: 'Count / sum',
  histogram: 'Histogram',
  exponential_histogram: 'Exp. histogram',
  summary: 'Summary',
  span: 'From traces',
  derived: 'Derived',
};
