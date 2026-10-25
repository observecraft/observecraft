import { useNavigate } from 'react-router-dom';
import { ChevronDown } from 'lucide-react';
import { Menu } from '../../components/ui/Dropdown';
import { logout, logoutAll } from '../../api/auth/client';
import { useProfile } from './queries';

const signOut = (all: boolean) => void (all ? logoutAll() : logout()).finally(() => window.location.assign('/login'));

/** "Personal Settings ▾" / "Organization Settings ▾" bar with the org, site and session links. */
export function SettingsHeader({ kind }: { kind: 'personal' | 'organization' }) {
  const navigate = useNavigate();
  const profile = useProfile();
  return (
    <header className="settings-header">
      <Menu
        align="left"
        triggerClassName="settings-switch"
        trigger={
          <>
            {kind === 'personal' ? 'Personal Settings' : 'Organization Settings'} <ChevronDown size={18} />
          </>
        }
        items={[
          { label: 'Personal Settings', onClick: () => navigate('/account/profile') },
          { label: 'Organization Settings', onClick: () => navigate('/organization-settings/users') },
        ]}
      />
      <div className="settings-header-right">
        <span>
          Organization: <b>{profile.data?.org.name ?? '…'}</b>
        </span>
        <span>
          Site: <b>{window.location.host}</b>
        </span>
        {kind === 'personal' && (
          <button className="link-muted" onClick={() => window.confirm('Sign out of ObserveCraft on every device and browser?') && signOut(true)}>
            Log Out of All Web Sessions
          </button>
        )}
        <button className="link-muted" onClick={() => signOut(false)}>
          Log Out
        </button>
      </div>
    </header>
  );
}
