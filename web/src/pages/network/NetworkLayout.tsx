import { createContext, useContext, useState } from 'react';
import { NavLink, Outlet } from 'react-router-dom';
import { Filter, Network } from 'lucide-react';
import { TimePicker } from '../../components/shell/PageHeader';
import { usePopover } from '../../components/ui/Dropdown';
import './network.css';

interface TrafficFilter {
  showExternal: boolean;
  showUnresolved: boolean;
}

const TrafficCtx = createContext<TrafficFilter>({ showExternal: true, showUnresolved: false });

/** The "Filter Traffic" toggles shared by Analytics and the Map. */
export const useTraffic = () => useContext(TrafficCtx);

const TABS = [
  { to: '/network', label: 'Analytics', end: true },
  { to: '/network/map', label: 'Map' },
];

function Toggle({ label, help, on, set }: { label: string; help: string; on: boolean; set: (v: boolean) => void }) {
  return (
    <label className="net-toggle" title={help}>
      <input type="checkbox" checked={on} onChange={(e) => set(e.target.checked)} />
      <span>{label}</span>
    </label>
  );
}

/** Cloud Network Monitoring: Analytics / Map, Filter Traffic and the time range. */
export function NetworkLayout() {
  const [showExternal, setShowExternal] = useState(true);
  const [showUnresolved, setShowUnresolved] = useState(false);
  const { open, setOpen, ref } = usePopover();
  return (
    <TrafficCtx.Provider value={{ showExternal, showUnresolved }}>
      <div className="net-layout">
        <header className="product-header">
          <div className="product-title">
            <Network size={20} />
            Cloud Network Monitoring
          </div>
          <nav className="product-tabs">
            {TABS.map((t) => (
              <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
                {t.label}
              </NavLink>
            ))}
          </nav>
          <div className="net-header-right">
            <div className="dd" ref={ref}>
              <button className="btn" onClick={() => setOpen((o) => !o)}>
                <Filter size={14} /> Filter Traffic
              </button>
              {open && (
                <div className="dd-menu net-filter-menu">
                  <Toggle label="Show External Traffic" help="Traffic to public addresses outside your network" on={showExternal} set={setShowExternal} />
                  <Toggle
                    label="Show N/A (Unresolved Traffic)"
                    help="Traffic whose client or server has no value for the grouping tag, such as hosts without an Agent"
                    on={showUnresolved}
                    set={setShowUnresolved}
                  />
                </div>
              )}
            </div>
            <TimePicker />
          </div>
        </header>
        <div className="net-section">
          <Outlet />
        </div>
      </div>
    </TrafficCtx.Provider>
  );
}
