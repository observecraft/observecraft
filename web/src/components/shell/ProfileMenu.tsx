import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { Check, ChevronDown, ChevronRight, Search } from 'lucide-react';
import { API_MODE } from '../../api/http';
import { logout, myOrgs, switchOrg } from '../../api/auth/client';
import { getProfile } from '../../api/identity/client';
import { usePopover } from '../ui/Dropdown';
import { UserAvatar } from '../ui/UserAvatar';

const LINKS: { label: string; to: string; admin?: boolean }[] = [
  { label: 'Personal Settings', to: '/account/profile' },
  { label: 'Organization Settings', to: '/organization-settings/users' },
  { label: 'Audit Trail', to: '/organization-settings/audit-trail' },
  { label: 'Sensitive Data Scanner', to: '/organization-settings/sensitive-data-scanner' },
  { label: 'Integrations', to: '/integrations' },
  { label: 'Instance Admin', to: '/instance/orgs', admin: true },
];

/** The account panel behind the avatar: who you are, the current organization, other orgs and your teams, settings links, log out. */
export function ProfileMenu() {
  const navigate = useNavigate();
  const { open, setOpen, ref } = usePopover();
  const [filter, setFilter] = useState('');
  const [teamsOpen, setTeamsOpen] = useState(false);
  // Same key as the settings pages' useProfile, so the profile is fetched once.
  const me = useQuery({ queryKey: ['identity', 'profile'], queryFn: getProfile, staleTime: 60_000, retry: false });
  const orgs = useQuery({ queryKey: ['identity', 'orgs'], queryFn: myOrgs, enabled: API_MODE === 'live', staleTime: 60_000, retry: false });

  const f = filter.trim().toLowerCase();
  const matches = (s: string) => !f || s.toLowerCase().includes(f);
  const current = orgs.data?.find((o) => o.current) ?? me.data?.org;
  const others = (orgs.data ?? []).filter((o) => !o.current && matches(o.name));
  const teams = (me.data?.teams ?? []).filter((t) => matches(t.name) || matches(t.handle));

  const go = (to: string) => {
    setOpen(false);
    navigate(to);
  };
  // A full reload drops every cached query of the previous org.
  const switchTo = (org: string) =>
    void switchOrg(org)
      .then(() => window.location.assign('/'))
      .catch((e: Error) => window.alert(e.message));

  return (
    <div className="pm" ref={ref}>
      <button type="button" className="topbar-account" onClick={() => setOpen((o) => !o)} aria-label="Account" aria-expanded={open}>
        {me.data ? <UserAvatar id={me.data.id} name={me.data.name} size={26} /> : <div className="avatar" />}
      </button>
      {open && (
        <div className="pm-panel" role="menu">
          {me.data && (
            <div className="pm-user">
              <UserAvatar id={me.data.id} name={me.data.name} size={24} />
              <div>
                <div className="pm-user-name">{me.data.name}</div>
                <div className="pm-user-email">{me.data.email}</div>
              </div>
            </div>
          )}
          {current && (
            <div className="pm-section">
              <div className="pm-label">Current organization</div>
              <div className="pm-org current">
                <span>{current.name}</span>
                <Check size={14} className="pm-check" />
              </div>
            </div>
          )}
          <div className="pm-search">
            <Search size={13} />
            <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Search orgs and teams…" aria-label="Search orgs and teams" />
          </div>
          {others.length > 0 && (
            <div className="pm-section pm-orgs">
              {others.map((o) => (
                <button key={o.id} type="button" className="pm-org" onClick={() => switchTo(o.id)}>
                  {o.name}
                </button>
              ))}
            </div>
          )}
          <button type="button" className="pm-teams-toggle" onClick={() => setTeamsOpen((v) => !v)} aria-expanded={teamsOpen || !!f}>
            <span>
              My Teams <span className="pm-count">{me.data?.teams.length ?? 0}</span>
            </span>
            {teamsOpen || f ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </button>
          {(teamsOpen || f) && (
            <div className="pm-teams">
              {teams.map((t) => (
                <button key={t.handle} type="button" className="pm-item" onClick={() => go('/teams')}>
                  {t.name} <span className="pm-handle">@{t.handle}</span>
                </button>
              ))}
              {!teams.length && <div className="pm-none">{f ? 'No matching teams' : 'You are not in a team yet'}</div>}
            </div>
          )}
          <div className="pm-divider" />
          {LINKS.filter((l) => !l.admin || me.data?.isInstanceAdmin).map((l) => (
            <button key={l.to} type="button" className="pm-item" role="menuitem" onClick={() => go(l.to)}>
              {l.label}
            </button>
          ))}
          {API_MODE === 'live' && (
            <>
              <div className="pm-divider" />
              <button type="button" className="pm-item pm-logout" role="menuitem" onClick={() => void logout().finally(() => window.location.assign('/login'))}>
                Log out
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
