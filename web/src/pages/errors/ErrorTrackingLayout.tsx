import { NavLink, Outlet } from 'react-router-dom';
import { Bug, Settings } from 'lucide-react';
import { TimeProvider } from '../../components/shell/TimeContext';

/** Error Tracking product frame. Its time range defaults to the past day, separate from Logs. */
export function ErrorTrackingLayout() {
  return (
    <TimeProvider initial="1d">
      <div className="et-layout">
        <header className="product-header">
          <div className="product-title">
            <Bug size={20} />
            Error Tracking
          </div>
          <nav className="product-tabs">
            <NavLink to="/errors" end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              Issues
            </NavLink>
          </nav>
          <button className="btn product-config">
            <Settings size={14} /> Settings
          </button>
        </header>
        <div className="et-section">
          <Outlet />
        </div>
      </div>
    </TimeProvider>
  );
}
