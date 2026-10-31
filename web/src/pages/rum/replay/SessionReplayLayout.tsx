import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { ChevronDown, PlayCircle } from 'lucide-react';
import { Menu } from '../../../components/ui/Dropdown';
import { ApplicationSelector } from '../RumLayout';
import { useSelectedApplication } from '../common';
import '../../monitors/monitors.css';
import '../rum.css';
import './replay.css';

/** Session Replay's frame: product switcher, application selector and its tabs. */
export function SessionReplayLayout() {
  const navigate = useNavigate();
  const [selected] = useSelectedApplication();
  const keep = (to: string) => (selected ? `${to}?application=${selected}` : to);
  const tab = ({ isActive }: { isActive: boolean }) => `product-tab${isActive ? ' active' : ''}`;
  return (
    <div className="mon-layout rum-layout">
      <header className="product-header rum-header">
        <Menu
          align="left"
          triggerClassName="product-title rum-switch"
          trigger={
            <>
              <PlayCircle size={20} /> Session Replay <ChevronDown size={16} />
            </>
          }
          items={[
            { label: 'Real User Monitoring', onClick: () => navigate('/rum') },
            { label: 'Session Replay', onClick: () => navigate('/rum/replay/sessions') },
            { label: 'Product Analytics', onClick: () => navigate('/product-analytics') },
            { label: 'Synthetic Monitoring & Testing', onClick: () => navigate('/synthetics') },
          ]}
        />
        <ApplicationSelector />
        <nav className="product-tabs">
          <NavLink to={keep('/rum/replay/sessions')} end className={tab}>
            Search Recordings
          </NavLink>
          <NavLink to={keep('/rum/replay/playlists')} className={tab}>
            Playlists
          </NavLink>
          <NavLink to={keep('/rum/replay/heatmaps')} className={tab}>
            Heatmaps
          </NavLink>
        </nav>
      </header>
      <div className="mon-section">
        <Outlet />
      </div>
    </div>
  );
}
