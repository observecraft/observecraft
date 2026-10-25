import { useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { ArrowUpDown, Lock, LockOpen, Plus, Users, X } from 'lucide-react';
import type { TeamSearchRequest } from '../../api/identity/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { Pager } from '../../components/ui/Pager';
import { AvatarStack } from '../../components/ui/UserAvatar';
import { useCreateTeam, useTeams } from './queries';
import './settings.css';

const VIA_LABEL: Record<string, string> = { ui: 'ObserveCraft UI', api: 'API', saml: 'SAML', terraform: 'Terraform' };

type Tab = 'list' | 'sources' | 'settings';

export function TeamsPage() {
  const [params, setParams] = useSearchParams();
  const [tab, setTab] = useState<Tab>('list');
  const [creating, setCreating] = useState(false);
  const [restriction, setRestriction] = useState('*');
  const [member, setMember] = useState('*');
  const [notif, setNotif] = useState('*');
  const [via, setVia] = useState('*');
  const [sort, setSort] = useState<TeamSearchRequest['sort']>('name');
  const [limit, setLimit] = useState('20');
  const [offset, setOffset] = useState(0);
  const query = params.get('q') ?? '';

  const req: TeamSearchRequest = {
    query,
    member: member === '*' ? null : member,
    restricted: restriction === '*' ? null : restriction === 'restricted',
    createdVia: via === '*' ? null : via,
    hasNotifications: notif === '*' ? null : notif === 'yes',
    sort,
    offset,
    limit: Number(limit),
  };
  const q = useTeams(req);
  const set = <T,>(fn: (v: T) => void) => (v: T) => {
    fn(v);
    setOffset(0);
  };

  return (
    <div className="teams-page">
      <header className="product-header">
        <div className="product-title">
          <Users size={20} /> Teams
        </div>
        <nav className="product-tabs">
          <button className={`product-tab${tab === 'list' ? ' active' : ''}`} onClick={() => setTab('list')}>
            Team List {q.data && <span className="teams-count">{q.data.total}</span>}
          </button>
          <button className={`product-tab${tab === 'sources' ? ' active' : ''}`} onClick={() => setTab('sources')}>
            Data Sources
          </button>
          <button className={`product-tab${tab === 'settings' ? ' active' : ''}`} onClick={() => setTab('settings')}>
            Settings
          </button>
        </nav>
        <button className="btn btn-primary product-config" onClick={() => setCreating(true)}>
          <Plus size={14} /> New Team
        </button>
      </header>
      {tab !== 'list' ? (
        <div className="settings-pad">
          <section className="settings-card settings-empty">
            <p className="muted">{tab === 'sources' ? 'Team data sources (SAML, GitHub, PagerDuty sync) are configured from SAML Group Mappings for now.' : 'Team settings will appear here.'}</p>
          </section>
        </div>
      ) : (
        <div className="teams-body">
          <input
            className="teams-search"
            placeholder="Team name, handle, description…"
            value={query}
            onChange={(e) => {
              setParams(e.target.value ? { q: e.target.value } : {}, { replace: true });
              setOffset(0);
            }}
          />
          <div className="teams-filters">
            <Select label="membership restriction" value={restriction} onChange={set(setRestriction)} options={[{ value: '*', label: '*' }, { value: 'restricted', label: 'Restricted' }, { value: 'open', label: 'Open' }]} />
            <Select label="member" value={member} onChange={set(setMember)} options={[{ value: '*', label: '*' }, { value: 'me', label: 'Me' }]} />
            <Select label="notification channels" value={notif} onChange={set(setNotif)} options={[{ value: '*', label: '*' }, { value: 'yes', label: 'Configured' }, { value: 'no', label: 'None' }]} />
            <Select label="created via" value={via} onChange={set(setVia)} options={[{ value: '*', label: '*' }, ...Object.entries(VIA_LABEL).map(([value, label]) => ({ value, label }))]} />
          </div>
          <section className="teams-table-wrap">
            {q.isPending ? (
              <Loading label="Loading teams…" />
            ) : q.isError ? (
              <ErrorState error={q.error} onRetry={() => q.refetch()} />
            ) : (
              <>
                <div className="results-bar">
                  <b>
                    {q.data.total === 0 ? 0 : offset + 1}–{offset + q.data.teams.length} of {q.data.total} teams
                  </b>
                  <Menu
                    trigger={
                      <>
                        <ArrowUpDown size={15} /> Sort By
                      </>
                    }
                    triggerClassName="link-muted"
                    items={[
                      { label: `Name${sort === 'name' ? ' ✓' : ''}`, onClick: () => set(setSort)('name') },
                      { label: `Members${sort === 'members' ? ' ✓' : ''}`, onClick: () => set(setSort)('members') },
                    ]}
                  />
                </div>
                <table className="settings-table teams-table">
                  <thead>
                    <tr>
                      <th>NAME</th>
                      <th className="col-lock" />
                      <th>MEMBERS</th>
                      <th>SUBTEAMS</th>
                      <th>CREATED VIA</th>
                      <th>NOTIFICATIONS</th>
                    </tr>
                  </thead>
                  <tbody>
                    {q.data.teams.map((t) => (
                      <tr key={t.handle}>
                        <td>
                          <div className="team-name">{t.name}</div>
                          <div className="muted">{t.handle}</div>
                        </td>
                        <td className="col-lock" title={t.restricted ? 'Only team admins can add members' : 'Anyone can join'}>
                          {t.restricted ? <Lock size={18} /> : <LockOpen size={18} />}
                        </td>
                        <td>{t.memberCount > 0 ? <AvatarStack users={t.members} total={t.memberCount} /> : <span className="muted">—</span>}</td>
                        <td>{t.subteams.join(', ') || <span className="muted">—</span>}</td>
                        <td>{VIA_LABEL[t.createdVia] ?? t.createdVia}</td>
                        <td>{t.notifications.join(', ') || <span className="muted">—</span>}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <div className="teams-pager">
                  <span>Results per page:</span>
                  <Select value={limit} onChange={set(setLimit)} options={['20', '50', '100'].map((v) => ({ value: v, label: v }))} />
                  <Pager offset={offset} limit={Number(limit)} total={q.data.total} onChange={setOffset} />
                </div>
              </>
            )}
          </section>
        </div>
      )}
      {creating && <NewTeam onClose={() => setCreating(false)} />}
    </div>
  );
}

function NewTeam({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState('');
  const [handle, setHandle] = useState('');
  const [touched, setTouched] = useState(false);
  const [description, setDescription] = useState('');
  const create = useCreateTeam();
  const autoHandle = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
  const h = touched ? handle : autoHandle;
  return (
    <Modal onClose={onClose} label="New team" width={520} height="auto">
      <form
        className="small-modal"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate({ name, handle: h, description }, { onSuccess: onClose });
        }}
      >
        <div className="modal-title-row">
          <h3>New Team</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} required maxLength={120} />
        </label>
        <label className="form-label">
          Handle
          <input
            value={h}
            onChange={(e) => {
              setTouched(true);
              setHandle(e.target.value);
            }}
            required
            maxLength={64}
          />
        </label>
        <label className="form-label">
          Description
          <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
        <p className="muted">You'll be the team's first admin.</p>
        {create.isError && <p className="form-error">{(create.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!name.trim() || !h || create.isPending}>
            Create Team
          </button>
        </div>
      </form>
    </Modal>
  );
}
