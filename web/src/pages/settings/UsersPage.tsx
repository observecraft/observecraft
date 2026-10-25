import { useMemo, useState } from 'react';
import { ChevronDown, ChevronRight, LayoutGrid, List, Search, UserPlus, X } from 'lucide-react';
import type { UserStatus, UserSummary } from '../../api/identity/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { Pager } from '../../components/ui/Pager';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { timeAgo } from '../../lib/format';
import { loginLink } from '../../api/auth/client';
import { useInviteUsers, useProfile, useRoles, useUpdateUser, useUsers } from './queries';

const PAGE = 50;
const STATUSES: UserStatus[] = ['active', 'pending', 'deactivated'];
const METHODS = [
  ['password', 'Password'],
  ['google', 'Google'],
  ['saml', 'SAML'],
] as const;
const LOGIN_LABEL: Record<string, string> = Object.fromEntries(METHODS);

function toggle<T>(list: T[], v: T): T[] {
  return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
}

function Facet({ title, children }: { title: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <div className="uf-group">
      <button className="uf-title" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />} {title}
      </button>
      {open && <div className="uf-body">{children}</div>}
    </div>
  );
}

function Check({ checked, onChange, label }: { checked: boolean; onChange: () => void; label: string }) {
  return (
    <label className="uf-row">
      <input type="checkbox" className="checkbox" checked={checked} onChange={onChange} />
      <span className="ellipsis">{label}</span>
    </label>
  );
}

export function UsersPage() {
  const [query, setQuery] = useState('');
  // Deactivated users are hidden by default, as in most admin consoles.
  const [statuses, setStatuses] = useState<UserStatus[]>(['active', 'pending']);
  const [methods, setMethods] = useState<string[]>(['password', 'google', 'saml']);
  const [mfa, setMfa] = useState<string[]>(['configured', 'unconfigured']);
  const [roleIds, setRoleIds] = useState<string[]>([]);
  const [roleFilter, setRoleFilter] = useState('');
  const [offset, setOffset] = useState(0);
  const [grid, setGrid] = useState(false);
  const [inviting, setInviting] = useState(false);
  const roles = useRoles();
  const me = useProfile();

  const req = { query, statuses, loginMethods: methods.length === 3 ? [] : methods, mfa, roleIds, offset, limit: PAGE };
  const q = useUsers(req);
  const reset = <T,>(set: (v: T) => void) => (v: T) => {
    set(v);
    setOffset(0);
  };

  const shownRoles = useMemo(() => (roles.data ?? []).filter((r) => r.name.toLowerCase().includes(roleFilter.trim().toLowerCase())), [roles.data, roleFilter]);
  const isAdmin = me.data?.roles.some((r) => r.id.endsWith('-admin')) ?? false;

  return (
    <div className="settings-pad users-page">
      <div className="settings-title-row">
        <h1 className="settings-title">Users</h1>
        {isAdmin ? (
          <button className="btn btn-primary" onClick={() => setInviting(true)}>
            <UserPlus size={14} /> Invite Users
          </button>
        ) : (
          <span className="read-only" title="Only admins can manage users">
            Read Only
          </span>
        )}
      </div>
      <label className="settings-search wide">
        <Search size={18} />
        <input placeholder="Filter users by name or email" value={query} onChange={(e) => reset(setQuery)(e.target.value)} />
      </label>
      <div className="users-layout">
        <aside className="uf-panel">
          <Facet title="Status">
            {STATUSES.map((s) => (
              <Check key={s} checked={statuses.includes(s)} onChange={() => reset(setStatuses)(toggle(statuses, s))} label={s[0].toUpperCase() + s.slice(1)} />
            ))}
          </Facet>
          <Facet title="Login Methods">
            {METHODS.map(([v, l]) => (
              <Check key={v} checked={methods.includes(v)} onChange={() => reset(setMethods)(toggle(methods, v))} label={l} />
            ))}
          </Facet>
          <Facet title="MFA">
            <Check checked={mfa.includes('configured')} onChange={() => reset(setMfa)(toggle(mfa, 'configured'))} label="Configured" />
            <Check checked={mfa.includes('unconfigured')} onChange={() => reset(setMfa)(toggle(mfa, 'unconfigured'))} label="Unconfigured" />
          </Facet>
          <Facet title="Roles">
            <label className="settings-search small">
              <Search size={14} />
              <input placeholder="Filter by name" value={roleFilter} onChange={(e) => setRoleFilter(e.target.value)} />
            </label>
            <div className="uf-scroll">
              {shownRoles.map((r) => (
                <Check key={r.id} checked={roleIds.includes(r.id)} onChange={() => reset(setRoleIds)(toggle(roleIds, r.id))} label={r.name} />
              ))}
            </div>
          </Facet>
        </aside>
        <section className="users-results">
          {q.isPending ? (
            <Loading label="Loading users…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <>
              <div className="results-bar">
                <span>
                  Showing <b>{q.data.total === 0 ? 0 : offset + 1}–{offset + q.data.users.length}</b> of <b>{q.data.total}</b> Users
                </span>
                <button className="btn" onClick={() => setGrid((g) => !g)}>
                  {grid ? <List size={14} /> : <LayoutGrid size={14} />} {grid ? 'List View' : 'Grid View'}
                </button>
              </div>
              {grid ? <UserGrid users={q.data.users} /> : <UserTable users={q.data.users} isAdmin={isAdmin} meId={me.data?.id} />}
              <Pager offset={offset} limit={PAGE} total={q.data.total} onChange={setOffset} />
            </>
          )}
        </section>
      </div>
      {inviting && <InviteModal onClose={() => setInviting(false)} />}
    </div>
  );
}

function RoleCell({ roles }: { roles: UserSummary['roles'] }) {
  if (roles.length === 0) return <span className="muted">—</span>;
  return (
    <span className="role-cell" title={roles.map((r) => r.name).join(', ')}>
      <span className="role-chip ellipsis">{roles[0].name}</span>
      {roles.length > 1 && <span className="more-chip">+{roles.length - 1}</span>}
    </span>
  );
}

const BUILT_IN = [
  { suffix: '-admin', label: 'Admin' },
  { suffix: '-standard', label: 'Standard' },
  { suffix: '-readonly', label: 'Read Only' },
];

/** Replaces the user's built-in role, keeping custom roles. */
function withBuiltIn(u: UserSummary, suffix: string): string[] {
  const org = u.roles.find((r) => BUILT_IN.some((b) => r.id.endsWith(b.suffix)))?.id.replace(/-(admin|standard|readonly)$/, '') ?? 'default';
  return [...u.roles.filter((r) => !BUILT_IN.some((b) => r.id === `${org}${b.suffix}`)).map((r) => r.id), `${org}${suffix}`];
}

function UserTable({ users, isAdmin, meId }: { users: UserSummary[]; isAdmin: boolean; meId?: string }) {
  const update = useUpdateUser();
  const [link, setLink] = useState<{ name: string; url: string } | null>(null);
  const [linkError, setLinkError] = useState<string | null>(null);
  const onLink = (u: UserSummary) => {
    setLinkError(null);
    loginLink(u.id)
      .then((r) => setLink({ name: u.name, url: r.url }))
      .catch((e: Error) => setLinkError(e.message));
  };
  return (
    <>
      {(update.error || linkError) && <p className="form-error">{linkError ?? (update.error as Error).message}</p>}
      {link && <LinkModal title={`Login link for ${link.name}`} links={[{ label: link.name, url: link.url }]} note="Valid for a single use. Share it privately — anyone with the link can set this user's password." onClose={() => setLink(null)} />}
      <table className="settings-table users-table">
        <thead>
          <tr>
            <th className="col-bar" />
            <th>NAME</th>
            <th>EMAIL</th>
            <th>ROLES</th>
            <th>LOGIN METHODS</th>
            <th>MFA</th>
            {isAdmin && <th />}
          </tr>
        </thead>
        <tbody>
          {users.map((u) => (
            <tr key={u.id} className={u.status === 'deactivated' ? 'row-muted' : ''}>
              <td className="col-bar">
                <i className={`user-bar user-${u.status}`} title={u.status} />
              </td>
              <td>
                <span className="user-name">
                  <UserAvatar id={u.id} name={u.name} size={22} />
                  {u.name}
                  {u.status === 'pending' && <span className="pill-chip small">Pending</span>}
                </span>
              </td>
              <td>{u.email}</td>
              <td className="col-roles">
                <RoleCell roles={u.roles} />
              </td>
              <td>{u.loginMethods.map((m) => LOGIN_LABEL[m] ?? m).join(', ')}</td>
              <td title={u.lastLoginAt ? `Last login ${timeAgo(u.lastLoginAt)}` : 'Never logged in'}>{u.mfa ? 'Configured' : 'Unconfigured'}</td>
              {isAdmin && (
                <td className="col-actions">
                  {u.id !== meId && (
                    <Menu
                      trigger={<span aria-label="User actions">⋯</span>}
                      triggerClassName="icon-plain"
                      items={[
                        ...(u.status !== 'deactivated'
                          ? [
                              { label: u.status === 'pending' ? 'Copy invitation link' : 'Copy login link (password reset)', onClick: () => onLink(u) },
                              ...BUILT_IN.filter((b) => !u.roles.some((r) => r.id.endsWith(b.suffix))).map((b) => ({ label: `Make ${b.label}`, onClick: () => update.mutate({ id: u.id, roleIds: withBuiltIn(u, b.suffix) }) })),
                              { label: 'Deactivate user', onClick: () => window.confirm(`Deactivate ${u.name}? They are signed out everywhere.`) && update.mutate({ id: u.id, status: 'deactivated' }) },
                            ]
                          : [{ label: 'Reactivate user', onClick: () => update.mutate({ id: u.id, status: 'active' }) }]),
                      ]}
                    />
                  )}
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

export function LinkModal({ title, links, note, onClose }: { title: string; links: { label: string; url: string }[]; note: string; onClose: () => void }) {
  const [copied, setCopied] = useState<string | null>(null);
  return (
    <Modal onClose={onClose} label={title} width={640} height="auto">
      <div className="small-modal">
        <div className="modal-title-row">
          <h3>{title}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <p className="muted">{note}</p>
        {links.map((l) => (
          <div key={l.url} className="link-row">
            <span className="link-label">{l.label}</span>
            <code className="link-url">{l.url}</code>
            <button
              className="btn btn-sm"
              onClick={() => {
                void navigator.clipboard?.writeText(l.url);
                setCopied(l.url);
              }}
            >
              {copied === l.url ? 'Copied' : 'Copy'}
            </button>
          </div>
        ))}
        <div className="small-modal-actions">
          <button className="btn btn-primary" onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    </Modal>
  );
}

function UserGrid({ users }: { users: UserSummary[] }) {
  return (
    <div className="user-grid">
      {users.map((u) => (
        <div key={u.id} className="user-card">
          <UserAvatar id={u.id} name={u.name} size={44} />
          <b className="ellipsis">{u.name}</b>
          <span className="muted ellipsis">{u.email}</span>
          <RoleCell roles={u.roles} />
        </div>
      ))}
    </div>
  );
}

function InviteModal({ onClose }: { onClose: () => void }) {
  const [emails, setEmails] = useState('');
  const [role, setRole] = useState('');
  const roles = useRoles();
  const invite = useInviteUsers();
  const [sent, setSent] = useState<{ email: string; url: string }[] | null>(null);
  const list = emails
    .split(/[\s,;]+/)
    .map((e) => e.trim())
    .filter(Boolean);
  if (sent)
    return (
      <LinkModal
        title={sent.length ? 'Invitations created' : 'Nobody new to invite'}
        links={sent.map((i) => ({ label: i.email, url: i.url }))}
        note={sent.length ? 'Send each person their link (valid 72 hours, single use). They choose a password — or sign in with SSO if your organization uses it.' : 'Those email addresses already have accounts.'}
        onClose={onClose}
      />
    );
  return (
    <Modal onClose={onClose} label="Invite users" width={520} height="auto">
      <form
        className="small-modal"
        onSubmit={(e) => {
          e.preventDefault();
          invite.mutate({ emails: list, roleIds: role ? [role] : [] }, { onSuccess: (r) => setSent(r as { email: string; url: string }[]) });
        }}
      >
        <div className="modal-title-row">
          <h3>Invite Users</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Emails
          <textarea rows={3} placeholder="name@example.com, other@example.com" value={emails} onChange={(e) => setEmails(e.target.value)} />
        </label>
        <label className="form-label">
          Role
          <select className="settings-input" value={role} onChange={(e) => setRole(e.target.value)}>
            <option value="">No role</option>
            {(roles.data ?? []).map((r) => (
              <option key={r.id} value={r.id}>
                {r.name}
              </option>
            ))}
          </select>
        </label>
        <p className="muted">Invited users appear as Pending until they accept. You'll get a link to send each of them.</p>
        {invite.isError && <p className="form-error">{(invite.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={list.length === 0 || invite.isPending}>
            Invite {list.length > 0 ? list.length : ''}
          </button>
        </div>
      </form>
    </Modal>
  );
}
