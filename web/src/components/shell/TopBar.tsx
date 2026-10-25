import { useNavigate } from 'react-router-dom';
import { Search, Sparkles } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { API_MODE } from '../../api/http';
import { logout } from '../../api/auth/client';
import { getProfile } from '../../api/identity/client';
import { Menu } from '../ui/Dropdown';
import { UserAvatar } from '../ui/UserAvatar';
import { ShowNavButton } from './Sidebar';

export function TopBar({ onShowNav }: { onShowNav?: () => void }) {
  const navigate = useNavigate();
  // Same key as the settings pages' useProfile, so the profile is fetched once.
  const me = useQuery({ queryKey: ['identity', 'profile'], queryFn: getProfile, enabled: API_MODE === 'live', staleTime: 60_000, retry: false });
  return (
    <header className="topbar">
      {onShowNav && <ShowNavButton onClick={onShowNav} />}
      <div className="topbar-search">
        <Search size={14} />
        <span>Search ObserveCraft</span>
        <kbd>Ctrl</kbd>
        <kbd>K</kbd>
      </div>
      <div className="topbar-right">
        <Menu
          trigger={me.data ? <UserAvatar id={me.data.id} name={me.data.name} size={26} /> : <div className="avatar" title="Account" />}
          triggerClassName="topbar-account"
          items={[
            { label: me.data ? `${me.data.name} · ${me.data.org.name}` : 'Account', onClick: () => navigate('/account/profile'), disabled: !me.data },
            { label: 'Personal Settings', onClick: () => navigate('/account/profile') },
            { label: 'Organization Settings', onClick: () => navigate('/organization-settings/users') },
            { label: 'Teams', onClick: () => navigate('/teams') },
            ...(API_MODE === 'live' ? [{ label: 'Log Out', onClick: () => void logout().finally(() => window.location.assign('/login')) }] : []),
          ]}
        />
        <button className="ask-btn">
          <Sparkles size={13} />
          Ask Assist
        </button>
      </div>
    </header>
  );
}
