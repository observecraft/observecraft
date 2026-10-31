import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { AtSign, Bell, Briefcase, Clock, LayoutDashboard, MailCheck, Pencil } from 'lucide-react';
import { myOrgs, switchOrg } from '../../api/auth/client';
import type { Profile } from '../../api/identity/client';
import { getEmailSubscriptions, sendEmailVerification, unsubscribeReport, type Preferences } from '../../api/identity/personal';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { formatLongDate, timeAgo } from '../../lib/format';
import { SettingsHeader } from './SettingsHeader';
import { useProfile, useUpdateProfile } from './queries';
import { usePreferences, useSavePreferences } from './preferences';
import { AppKeysTab, SecuritySection } from './SecuritySettings';
import './settings.css';

type Section = 'profile' | 'security' | 'home' | 'time' | 'accessibility' | 'email' | 'orgs' | 'mobile';
type Tab = 'overview' | 'oncall' | 'agent' | 'keys';

const SECTIONS: { id: Exclude<Section, 'profile'>; label: string; badge?: string }[] = [
  { id: 'home', label: 'Home Page', badge: 'NEW' },
  { id: 'time', label: 'Time Preferences' },
  { id: 'accessibility', label: 'Accessibility' },
  { id: 'email', label: 'Email Subscriptions' },
  { id: 'orgs', label: 'Organizations' },
  { id: 'mobile', label: 'Mobile and Third-Party Access' },
  { id: 'security', label: 'Security' },
];

const FALLBACK_TIMEZONES = ['Europe/Berlin', 'Europe/London', 'Europe/Warsaw', 'Asia/Jerusalem', 'America/New_York', 'America/Los_Angeles', 'Asia/Tokyo', 'UTC'];
const TIMEZONES: string[] = (() => {
  try {
    return ['UTC', ...(Intl as unknown as { supportedValuesOf(k: string): string[] }).supportedValuesOf('timeZone')];
  } catch {
    return FALLBACK_TIMEZONES;
  }
})();

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
            {SECTIONS.map((s) => (
              <button key={s.id} className={`settings-link${section === s.id ? ' active' : ''}`} onClick={() => setSection(s.id)}>
                {s.label} {s.badge && <span className="badge-new">{s.badge}</span>}
              </button>
            ))}
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
                <dt>Created</dt>
                <dd>{formatLongDate(p.createdAt).split(' at ')[0]}</dd>
              </div>
              <div>
                <dt>Last Modified</dt>
                <dd>{p.modifiedAt ? formatLongDate(p.modifiedAt).split(' at ')[0] : 'Never'}</dd>
              </div>
              <div>
                <dt>Email Verified</dt>
                <dd>
                  <VerifyEmail verified={p.emailVerified} />
                </dd>
              </div>
            </dl>
          </section>
        </div>
      ) : tab === 'keys' ? (
        <AppKeysTab />
      ) : (
        <section className="settings-card settings-empty">
          <h2>{tab === 'oncall' ? 'On-Call' : 'Agent Console'}</h2>
          {tab === 'oncall' ? (
            <p className="muted">
              Your shifts and how pages reach you are in <Link to="/on-call/profile">On-Call → My On-Call Profile</Link>.
            </p>
          ) : (
            <p className="muted">Manage the collectors you have access to from Fleet Automation.</p>
          )}
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

function VerifyEmail({ verified }: { verified: boolean }) {
  const [params] = useSearchParams();
  const send = useMutation({ mutationFn: sendEmailVerification });
  if (verified || params.get('verified') === '1') return <>Verified</>;
  return (
    <>
      Not Verified{' '}
      {send.isSuccess ? (
        <span className="muted">— link sent to {send.data.email}</span>
      ) : (
        <button className="link-button" disabled={send.isPending} onClick={() => send.mutate()}>
          Verify Email
        </button>
      )}
      {params.get('verified') === 'expired' && <span className="form-error"> The link expired: send a new one.</span>}
      {send.isError && <span className="form-error"> {(send.error as Error).message}</span>}
    </>
  );
}

function OtherSection({ section, p }: { section: Exclude<Section, 'profile' | 'security'>; p: Profile }) {
  const label = SECTIONS.find((s) => s.id === section)!.label;
  const prefs = usePreferences();
  return (
    <div className="settings-pad">
      <h1 className="settings-title">{label}</h1>
      {prefs.isPending && (section === 'home' || section === 'time' || section === 'accessibility') ? (
        <Loading />
      ) : section === 'home' ? (
        <HomeSection prefs={prefs.data!} />
      ) : section === 'time' ? (
        <TimeSection prefs={prefs.data!} p={p} />
      ) : section === 'accessibility' ? (
        <AccessibilitySection prefs={prefs.data!} />
      ) : section === 'email' ? (
        <EmailSection />
      ) : section === 'orgs' ? (
        <OrgsSection />
      ) : (
        <MobileSection />
      )}
    </div>
  );
}

const HOME_PAGES: [string, string][] = [
  ['/logs', 'Log Explorer'],
  ['/dashboards', 'Dashboards'],
  ['/apm', 'APM'],
  ['/infrastructure', 'Infrastructure'],
  ['/monitors', 'Monitors'],
  ['/incidents', 'Incidents'],
  ['/security', 'Security'],
];

function HomeSection({ prefs }: { prefs: Preferences }) {
  const save = useSavePreferences();
  const current = prefs.homePage ?? '/logs';
  const preset = HOME_PAGES.some(([v]) => v === current);
  const [custom, setCustom] = useState(preset ? '' : current);
  return (
    <section className="settings-card">
      <h2>Your home page</h2>
      <p className="muted">The page ObserveCraft opens when you sign in or select the logo.</p>
      <div className="pref-options">
        {HOME_PAGES.map(([v, l]) => (
          <label key={v} className="pref-radio">
            <input type="radio" name="home" checked={current === v} onChange={() => save.mutate({ ...prefs, homePage: v === '/logs' ? null : v })} /> {l}
          </label>
        ))}
        <label className="pref-radio">
          <input type="radio" name="home" checked={!preset} onChange={() => custom && save.mutate({ ...prefs, homePage: custom })} /> A page of your choice
        </label>
        <div className="pref-inline">
          <input className="settings-input" placeholder="/dashboards/storefront-overview" value={custom} onChange={(e) => setCustom(e.target.value)} aria-label="Home page path" />
          <button className="btn" disabled={!custom.trim() || save.isPending} onClick={() => save.mutate({ ...prefs, homePage: custom })}>
            Save
          </button>
        </div>
      </div>
      {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
    </section>
  );
}

function TimeSection({ prefs, p }: { prefs: Preferences; p: Profile }) {
  const save = useSavePreferences();
  const profile = useUpdateProfile();
  return (
    <>
      <section className="settings-card">
        <h2>Time zone</h2>
        <p className="muted">Shown as your local time on your profile and used by schedules you create. Charts follow your browser's time zone.</p>
        <Select label="Time zone" value={p.timezone ?? 'UTC'} options={TIMEZONES.map((t) => ({ value: t, label: t }))} onChange={(timezone) => profile.mutate({ timezone })} />
      </section>
      <section className="settings-card">
        <h2>Time format</h2>
        <div className="pref-options">
          {(
            [
              ['12h', '12-hour (2:30:00 pm)'],
              ['24h', '24-hour (14:30:00)'],
            ] as const
          ).map(([v, l]) => (
            <label key={v} className="pref-radio">
              <input type="radio" name="clock" checked={prefs.timeFormat === v} onChange={() => save.mutate({ ...prefs, timeFormat: v })} /> {l}
            </label>
          ))}
        </div>
        <h2>First day of the week</h2>
        <Select
          label="Week starts on"
          value={prefs.weekStart}
          options={[
            { value: 'monday', label: 'Monday' },
            { value: 'sunday', label: 'Sunday' },
          ]}
          onChange={(v) => save.mutate({ ...prefs, weekStart: v as Preferences['weekStart'] })}
        />
        {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
      </section>
    </>
  );
}

function AccessibilitySection({ prefs }: { prefs: Preferences }) {
  const save = useSavePreferences();
  const options: [keyof Preferences, string, string][] = [
    ['reducedMotion', 'Reduce motion', 'Turns off animations and transitions.'],
    ['highContrast', 'Increase contrast', 'Darker secondary text and borders, and a stronger focus outline.'],
    ['underlineLinks', 'Underline links', 'Tells links apart from text without relying on color.'],
    ['colorBlindPalette', 'Color-blind friendly charts', 'Chart colors that stay distinct with red–green and blue–yellow color blindness.'],
  ];
  return (
    <section className="settings-card">
      <h2>Display</h2>
      <p className="muted">Applies to every page, in every browser you sign in to.</p>
      <div className="pref-toggles">
        {options.map(([k, l, d]) => (
          <label key={k} className="pref-toggle">
            <input type="checkbox" role="switch" checked={Boolean(prefs[k])} onChange={(e) => save.mutate({ ...prefs, [k]: e.target.checked })} />
            <span>
              <b>{l}</b>
              <small className="muted">{d}</small>
            </span>
          </label>
        ))}
      </div>
      {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
    </section>
  );
}

const FREQ: Record<string, string> = { daily: 'Daily', weekly: 'Weekly', monthly: 'Monthly', hourly: 'Hourly' };

function EmailSection() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['identity', 'email-subscriptions'], queryFn: getEmailSubscriptions });
  const leave = useMutation({ mutationFn: unsubscribeReport, onSuccess: () => qc.invalidateQueries({ queryKey: ['identity', 'email-subscriptions'] }) });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const s = q.data;
  return (
    <>
      <section className="settings-card">
        <h2>Email address</h2>
        <div className="contact-row">
          <span className="contact-icon">
            <MailCheck size={20} />
          </span>
          <div>
            <div>{s.email}</div>
            <div className="muted">
              <VerifyEmail verified={s.verified} />
            </div>
          </div>
        </div>
        {!s.emailEnabled && <p className="muted">Email isn't configured on this install, so nothing is sent until an administrator sets OC_SMTP_URL.</p>}
      </section>
      <section className="settings-card">
        <h2>Dashboard reports</h2>
        <p className="muted">Scheduled reports sent to you. Unsubscribing removes you from the report's recipients.</p>
        {s.reports.length === 0 ? (
          <p className="muted">You don't receive any dashboard report.</p>
        ) : (
          <table className="settings-table">
            <thead>
              <tr>
                <th>REPORT</th>
                <th>DASHBOARD</th>
                <th>SCHEDULE</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {s.reports.map((r) => (
                <tr key={r.id}>
                  <td>{r.name}</td>
                  <td>
                    <Link to={`/dashboards/${r.dashboardId}`}>
                      <LayoutDashboard size={13} /> {r.dashboardTitle}
                    </Link>
                  </td>
                  <td>
                    {FREQ[String(r.schedule.frequency)] ?? String(r.schedule.frequency ?? '')} {r.schedule.time ? `at ${String(r.schedule.time)}` : ''}
                  </td>
                  <td className="num">
                    <button className="btn btn-sm" disabled={leave.isPending} onClick={() => leave.mutate(r.id)}>
                      Unsubscribe
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      <section className="settings-card">
        <h2>Monitor notifications</h2>
        <p className="muted">Monitors whose message mentions your address. Edit a monitor's message to stop its notifications.</p>
        {s.monitors.length === 0 ? (
          <p className="muted">No monitor notifies you by email.</p>
        ) : (
          <div className="chip-row">
            {s.monitors.map((m) => (
              <Link key={m.id} to={`/monitors/${m.id}`} className="pill-chip">
                <Bell size={12} /> {m.name}
              </Link>
            ))}
          </div>
        )}
      </section>
    </>
  );
}

function OrgsSection() {
  const q = useQuery({ queryKey: ['auth', 'my-orgs'], queryFn: myOrgs });
  const sw = useMutation({ mutationFn: switchOrg, onSuccess: () => window.location.assign('/') });
  return (
    <section className="settings-card">
      <h2>Your organizations</h2>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <table className="settings-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>ID</th>
              <th>ROLE</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {q.data.map((o) => (
              <tr key={o.id}>
                <td>{o.name}</td>
                <td className="mono">{o.id}</td>
                <td>{o.isAdmin ? 'Admin' : 'Member'}</td>
                <td className="num">
                  {o.current ? (
                    <span className="pill-chip">Current</span>
                  ) : (
                    <button className="btn btn-sm" disabled={sw.isPending} onClick={() => sw.mutate(o.id)}>
                      Switch
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {sw.isError && <p className="form-error">{(sw.error as Error).message}</p>}
    </section>
  );
}

function MobileSection() {
  return (
    <>
      <section className="settings-card">
        <h2>Third-party access</h2>
        <p className="muted">
          Tools that act as you (Terraform, API clients, scripts) use your application keys. Revoke a key to cut off what uses it. ObserveCraft has no mobile app: open it in your phone's browser.
        </p>
      </section>
      <AppKeysTab />
    </>
  );
}
