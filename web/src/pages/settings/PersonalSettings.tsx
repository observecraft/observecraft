import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { AtSign, Briefcase, Clock, Pencil } from 'lucide-react';
import type { Profile } from '../../api/identity/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { formatLongDate, timeAgo } from '../../lib/format';
import { SettingsHeader } from './SettingsHeader';
import { useProfile, useUpdateProfile } from './queries';
import { AppKeysTab, SecuritySection } from './SecuritySettings';
import './settings.css';

type Section = 'profile' | 'security' | 'home' | 'time' | 'accessibility' | 'email' | 'orgs' | 'mobile';
type Tab = 'overview' | 'oncall' | 'agent' | 'keys';

/** `soon`: nothing to configure yet; listed (greyed) but not selectable. */
const SECTIONS: { id: Exclude<Section, 'profile'>; label: string; badge?: string; soon?: boolean }[] = [
  { id: 'security', label: 'Security' },
  { id: 'home', label: 'Home Page', soon: true },
  { id: 'time', label: 'Time Preferences' },
  { id: 'accessibility', label: 'Accessibility', soon: true },
  { id: 'email', label: 'Email Subscriptions', soon: true },
  { id: 'orgs', label: 'Organizations' },
  { id: 'mobile', label: 'Mobile and Third-Party Access', soon: true },
];

const TIMEZONES = ['Europe/Berlin', 'Europe/London', 'Europe/Warsaw', 'Asia/Jerusalem', 'America/New_York', 'America/Los_Angeles', 'Asia/Tokyo', 'UTC'];

const LOGIN_LABEL: Record<string, string> = { saml: 'SAML', google: 'Google', password: 'Password' };

function localTime(tz: string | null | undefined, now: number): string {
  try {
    return new Date(now).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit', timeZone: tz ?? undefined }).toLowerCase();
  } catch {
    return new Date(now).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' }).toLowerCase();
  }
}

export function PersonalSettings() {
  const q = useProfile();
  const [section, setSection] = useState<Section>('profile');
  return (
    <div className="settings-page">
      <SettingsHeader kind="personal" />
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="settings-body">
          <aside className="settings-nav">
            <button className={`profile-card${section === 'profile' ? ' active' : ''}`} onClick={() => setSection('profile')}>
              <UserAvatar id={q.data.id} name={q.data.name} size={52} />
              <span>
                <b>{q.data.name.toUpperCase()}</b>
                <small>Profile Settings</small>
              </span>
            </button>
            {SECTIONS.map((s) =>
              s.soon ? (
                <span key={s.id} className="settings-link settings-link-soon" title={`${s.label} isn't built yet`}>
                  {s.label}
                  <span className="badge-soon">SOON</span>
                </span>
              ) : (
                <button key={s.id} className={`settings-link${section === s.id ? ' active' : ''}`} onClick={() => setSection(s.id)}>
                  {s.label} {s.badge && <span className="badge-new">{s.badge}</span>}
                </button>
              ),
            )}
          </aside>
          <main className="settings-main">{section === 'profile' ? <ProfileView p={q.data} /> : section === 'security' ? <SecuritySection /> : <OtherSection section={section} p={q.data} />}</main>
        </div>
      )}
    </div>
  );
}

function ProfileView({ p }: { p: Profile }) {
  const [tab, setTab] = useState<Tab>('overview');
  const [editing, setEditing] = useState(false);
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);
  return (
    <>
      <div className="profile-banner" />
      <div className="profile-head">
        <UserAvatar id={p.id} name={p.name} size={112} />
        <div className="profile-id">
          <h1>{p.name.toUpperCase()}</h1>
          <div className="profile-meta">
            {p.title && (
              <span>
                <Briefcase size={15} /> {p.title}
              </span>
            )}
            <span>
              <Clock size={15} /> {localTime(p.timezone, now)} local time
            </span>
          </div>
        </div>
        <button className="btn btn-primary" onClick={() => setEditing(true)}>
          <Pencil size={14} /> Edit
        </button>
      </div>
      {editing && <EditProfile p={p} onClose={() => setEditing(false)} />}
      <nav className="settings-tabs">
        {(
          [
            ['overview', 'Overview'],
            ['oncall', 'On-Call'],
            ['agent', 'Agent Console'],
            ['keys', 'Application Keys'],
          ] as const
        ).map(([id, label]) => (
          <button key={id} className={tab === id ? 'active' : ''} onClick={() => setTab(id)}>
            {label} {id === 'agent' && <span className="badge-preview">PREVIEW</span>}
          </button>
        ))}
      </nav>
      {tab === 'overview' ? (
        <div className="profile-cards">
          <section className="settings-card">
            <h2>Contact Info</h2>
            <div className="contact-row">
              <span className="contact-icon">
                <AtSign size={20} />
              </span>
              <div>
                <div>{p.email}</div>
                <div className="muted">Email</div>
              </div>
            </div>
          </section>
          <section className="settings-card">
            <h2>Teams</h2>
            {p.teams.length === 0 ? (
              <p className="muted">Not a member of any team.</p>
            ) : (
              <div className="chip-row">
                {p.teams.map((t) => (
                  <Link key={t.handle} to={`/teams?q=${encodeURIComponent(t.handle)}`} className="pill-chip">
                    {t.handle}
                  </Link>
                ))}
              </div>
            )}
          </section>
          <section className="settings-card">
            <h2>Roles</h2>
            <div className="chip-row">
              {p.roles.map((r) => (
                <span key={r.id} className="role-chip">
                  {r.name}
                </span>
              ))}
            </div>
          </section>
          <section className="settings-card">
            <h2>Activity and Login</h2>
            <div className="status-line">
              ObserveCraft Status <span className={`status-pill status-${p.status}`}>{p.status.toUpperCase()}</span>
            </div>
            <dl className="activity-grid">
              <div>
                <dt>Last Login</dt>
                <dd>{p.lastLoginAt ? `${formatLongDate(p.lastLoginAt)} (${timeAgo(p.lastLoginAt)})` : 'Never'}</dd>
              </div>
              <div>
                <dt>Login Methods</dt>
                <dd>{p.loginMethods.map((m) => LOGIN_LABEL[m] ?? m).join(', ')}</dd>
              </div>
              <div>
                <dt>Multi-Factor Authentication</dt>
                <dd>{p.mfa ? 'Configured' : 'Unconfigured'}</dd>
              </div>
              <div>
                <dt>Member Since</dt>
                <dd>{formatLongDate(p.createdAt).split(' at ')[0]}</dd>
              </div>
            </dl>
          </section>
        </div>
      ) : tab === 'keys' ? (
        <AppKeysTab />
      ) : (
        <section className="settings-card settings-empty">
          <h2>{tab === 'oncall' ? 'Not on call' : 'Agent Console'}</h2>
          <p className="muted">{tab === 'oncall' ? 'You are not part of an on-call schedule.' : 'Manage the collectors you have access to from Fleet Automation.'}</p>
        </section>
      )}
    </>
  );
}

function EditProfile({ p, onClose }: { p: Profile; onClose: () => void }) {
  const [name, setName] = useState(p.name);
  const [title, setTitle] = useState(p.title ?? '');
  const save = useUpdateProfile();
  return (
    <form
      className="settings-card edit-profile"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate({ name, title }, { onSuccess: onClose });
      }}
    >
      <label>
        Name
        <input className="settings-input" value={name} onChange={(e) => setName(e.target.value)} maxLength={120} required />
      </label>
      <label>
        Title
        <input className="settings-input" value={title} onChange={(e) => setTitle(e.target.value)} maxLength={120} />
      </label>
      {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
      <div className="form-actions">
        <button type="button" className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn btn-primary" disabled={save.isPending || !name.trim()}>
          Save
        </button>
      </div>
    </form>
  );
}

function OtherSection({ section, p }: { section: Exclude<Section, 'profile' | 'security'>; p: Profile }) {
  const save = useUpdateProfile();
  const label = SECTIONS.find((s) => s.id === section)!.label;
  return (
    <div className="settings-pad">
      <h1 className="settings-title">{label}</h1>
      {section === 'time' ? (
        <section className="settings-card">
          <h2>Time zone</h2>
          <p className="muted">Used for "local time" on your profile. Charts follow your browser's time zone.</p>
          <Select label="Time zone" value={p.timezone ?? 'UTC'} options={TIMEZONES.map((t) => ({ value: t, label: t }))} onChange={(timezone) => save.mutate({ timezone })} />
        </section>
      ) : section === 'orgs' ? (
        <section className="settings-card">
          <h2>Your organizations</h2>
          <table className="settings-table">
            <thead>
              <tr>
                <th>NAME</th>
                <th>ID</th>
                <th />
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>{p.org.name}</td>
                <td className="mono">{p.org.id}</td>
                <td>
                  <span className="pill-chip">Current</span>
                </td>
              </tr>
            </tbody>
          </table>
        </section>
      ) : (
        <section className="settings-card settings-empty">
          <p className="muted">Nothing to configure here yet.</p>
        </section>
      )}
    </div>
  );
}
