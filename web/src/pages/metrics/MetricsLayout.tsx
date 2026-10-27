import { NavLink, Outlet } from 'react-router-dom';
import { Gauge } from 'lucide-react';
import { TimeProvider } from '../../components/shell/TimeContext';
import './metrics.css';

/** Metrics product frame: Explorer, Summary and Volume share one time range (past hour by default). */
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
            <NavLink to="/metrics" end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              Explorer
            </NavLink>
            <NavLink to="/metrics/summary" className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              Summary
            </NavLink>
            <NavLink to="/metrics/volume" className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              Volume
            </NavLink>
          </nav>
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
};
