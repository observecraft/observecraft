import { useState, type ReactNode } from 'react';
import { NavLink } from 'react-router-dom';
import { ChartLine, LayoutDashboard } from 'lucide-react';
import { QuickGraphModal } from '../QuickGraphModal';

/** Product header of the dashboard list, Shared Dashboards and Reports tabs, with Quick Graph. */
export function DashboardsHeader({ children, quickGraph, onQuickGraphClose }: { children?: ReactNode; quickGraph?: boolean; onQuickGraphClose?: () => void }) {
  const [open, setOpen] = useState(false);
  const shown = open || Boolean(quickGraph);
  const tab = (to: string, label: string) => (
    <NavLink to={to} end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
      {label}
    </NavLink>
  );
  return (
    <header className="product-header">
      <div className="product-title">
        <LayoutDashboard size={20} /> Dashboards
      </div>
      <nav className="product-tabs">
        {tab('/dashboards', 'List')}
        {tab('/dashboards/shared', 'Shared Dashboards')}
        {tab('/dashboards/reports', 'Reports')}
      </nav>
      <div className="dl-header-actions">
        <button className="btn" onClick={() => setOpen(true)} title="Graph a metric without creating a dashboard">
          <ChartLine size={14} /> Quick Graph
        </button>
        {children}
      </div>
      {shown && (
        <QuickGraphModal
          onClose={() => {
            setOpen(false);
            onQuickGraphClose?.();
          }}
        />
      )}
    </header>
  );
}
