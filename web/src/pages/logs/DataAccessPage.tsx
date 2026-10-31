import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, Pencil, Plus, Search, ShieldAlert, Trash2, Users, X } from 'lucide-react';
import {
  createRestrictionQuery,
  deleteRestrictionQuery,
  listRestrictionQueries,
  previewRestrictionQuery,
  unassignRole,
  unassignTeam,
  updateRestrictionQuery,
  type LogRestrictionQuery,
  type RestrictionQueryInput,
} from '../../api/logRestrictions/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { compact } from '../../lib/format';
import { useProfile, useRoles, useTeams, useUsers } from '../settings/queries';
import { Avatar, Columns, ConfigTitle, FilterQuery, fmtDate, SectionHead } from './config/configUi';

const KEY = ['log-restrictions'];
const TEAMS_REQUEST = { offset: 0, limit: 100, sort: 'name' } as const;

/** `value`, once it has stopped changing for `ms`. */
function useSettled<T>(value: T, ms = 400): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return settled;
}

/** Logs of the last hour a query matches (and whether it is valid), counted by the server. */
const usePreview = (query: string) => useQuery({ queryKey: [...KEY, 'preview', query], queryFn: () => previewRestrictionQuery(query), enabled: query !== '', retry: false, staleTime: 60_000 });

/** How the rules work; the server applies the same ones (crate::log_access). */
function Rules() {
  return (
    <ul className="muted da-rules">
      <li>While there are no restriction queries, everyone who can read logs sees all of the organization's logs.</li>
      <li>
        Once one exists, a user sees only the logs matching at least one of the queries assigned to one of their roles or teams. Users with no query assigned see <b>no logs</b>: access is granted,
        never assumed.
      </li>
      <li>Org admins always see every log. Background monitor evaluation reads every log too; previews, dashboards, live tail, error tracking and archive searches follow the viewer's access.</li>
    </ul>
  );
}

/** Log Configuration → Data Access: restriction queries limit which logs each role or team reads. */
export function DataAccessPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const org = me.data?.org.id;
  const isAdmin = me.data?.roles.some((r) => r.id === `${org}-admin`) ?? false;
  const q = useQuery({ queryKey: KEY, queryFn: listRestrictionQueries, enabled: isAdmin });
  const [editing, setEditing] = useState<LogRestrictionQuery | 'new' | null>(null);
  const [roleFilter, setRoleFilter] = useState('');
  const [viewAs, setViewAs] = useState('');
  const [search, setSearch] = useState('');
  const [open, setOpen] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);
  const roles = useRoles();
  const users = useUsers({ limit: 200, offset: 0, statuses: ['active'] });
  const needle = search.trim().toLowerCase();
  const refresh = () => qc.invalidateQueries({ queryKey: KEY });
  const remove = useMutation({ mutationFn: deleteRestrictionQuery, onSuccess: refresh });
  const dropRole = useMutation({ mutationFn: ({ id, roleId }: { id: string; roleId: string }) => unassignRole(id, roleId), onSuccess: refresh });
  const dropTeam = useMutation({ mutationFn: ({ id, handle }: { id: string; handle: string }) => unassignTeam(id, handle), onSuccess: refresh });

  if (me.isPending) return <Loading />;
  if (!isAdmin) {
    return (
      <div className="settings-pad">
        <h1 className="settings-title">Data Access</h1>
        <Rules />
        <section className="settings-card settings-empty">
          <p className="muted">Only organization admins manage data access.</p>
        </section>
      </div>
    );
  }
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const queries = q.data;
  const error = remove.error ?? dropRole.error ?? dropTeam.error;
  const viewer = viewAs ? users.data?.users.find((u) => u.id === viewAs) : undefined;
  const viewerRoles = new Set((viewer?.roles ?? []).map((r) => r.id));
  const isViewerAdmin = viewer?.roles.some((r) => r.id === `${org}-admin`) ?? false;
  const shown = queries.filter(
    (r) =>
      (!roleFilter || r.roles.some((x) => x.id === roleFilter)) &&
      (!viewer || isViewerAdmin || r.roles.some((x) => viewerRoles.has(x.id))) &&
      (!needle || r.name.toLowerCase().includes(needle) || r.query.toLowerCase().includes(needle)),
  );
  const template = '20px minmax(320px, 3fr) 90px 90px 110px 130px 34px 64px';
  return (
    <div className="lc-page">
      <ConfigTitle
        title="Data Access"
        actions={
          <button className="btn btn-primary" onClick={() => setEditing('new')}>
            <Plus size={14} /> New Query
          </button>
        }
      />
      <div className="lc-view">
        <label className="lc-view-select da-filter">
          <span>Role</span>
          <select value={roleFilter} onChange={(e) => (setRoleFilter(e.target.value), setViewAs(''))} aria-label="Filter by role">
            <option value="">Select roles to filter the list</option>
            {(roles.data ?? []).map((r) => (
              <option key={r.id} value={r.id}>
                {r.name}
              </option>
            ))}
          </select>
        </label>
        <span className="muted da-or">OR</span>
        <label className="lc-view-select">
          <span>
            <Users size={13} /> View As
          </span>
          <select value={viewAs} onChange={(e) => (setViewAs(e.target.value), setRoleFilter(''))} aria-label="View as user">
            <option value="">Anyone</option>
            {(users.data?.users ?? []).map((u) => (
              <option key={u.id} value={u.id}>
                {u.name || u.email}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div className="lc-count">
        <b>{queries.length}</b> restriction queries defined
      </div>
      {viewer && (
        <p className="lc-banner">
          {isViewerAdmin ? (
            <>
              <b>{viewer.name || viewer.email}</b> is an org admin and reads every log.
            </>
          ) : queries.length === 0 ? (
            <>No restriction queries: {viewer.name || viewer.email} reads every log.</>
          ) : shown.length === 0 ? (
            <>
              <b>{viewer.name || viewer.email}</b> reads <b>no logs</b>: none of their roles has a restriction query (team assignments also count, not shown here).
            </>
          ) : (
            <>
              <b>{viewer.name || viewer.email}</b> reads the logs matching any of the {shown.length} queries below (through their roles; team assignments add to it).
            </>
          )}
        </p>
      )}
      <Rules />
      {queries.length > 0 && (
        <p className="log-restricted-notice">
          <ShieldAlert size={14} /> Restrictions are on: users who aren't admins and have no query assigned see no logs.
        </p>
      )}
      {error && <p className="form-error">{(error as Error).message}</p>}
      <section className="lc-section">
        <SectionHead open={open} onToggle={() => setOpen((o) => !o)} label={`RESTRICTED ACCESS (${queries.length})`} />
        {open && (
          <>
            <div className="lc-detail da-search">
              <div className="lc-search">
                <Search size={14} className="muted" />
                <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="filter list" aria-label="Filter restriction queries" />
              </div>
            </div>
            <Columns template={template} cols={['', 'Query', '# of roles', '# of users', 'Logs (1h)', 'Last modified', '', '']} />
            {shown.map((r) => {
              const isOpen = expanded === r.id;
              return (
                <div key={r.id}>
                  <div className="lc-row" style={{ gridTemplateColumns: template }}>
                    <button className="icon-plain" onClick={() => setExpanded(isOpen ? null : r.id)} aria-expanded={isOpen} title="Roles and teams">
                      {isOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
                    </button>
                    <span className="lc-cell-name" title={r.name === r.query ? r.query : `${r.name}: ${r.query}`}>
                      <Link to={`/logs?query=${encodeURIComponent(r.query)}`}>
                        <FilterQuery query={r.query} />
                      </Link>
                      {r.name !== r.query && <span className="muted">{r.name}</span>}
                    </span>
                    <span>{r.roles.length + r.teams.length}</span>
                    <span>{r.userCount}</span>
                    <MatchCount query={r.query} />
                    <span className="muted">{fmtDate(r.updatedAt)}</span>
                    <Avatar name={r.createdBy} />
                    <span className="lc-actions">
                      <button className="icon-plain" title="Edit" onClick={() => setEditing(r)}>
                        <Pencil size={15} />
                      </button>
                      <button
                        className="icon-plain"
                        title="Delete"
                        onClick={() =>
                          window.confirm(
                            `Delete restriction query "${r.name}"? Its users lose the access it grants${queries.length === 1 ? '; with no queries left, everyone sees all logs again' : ''}.`,
                          ) && remove.mutate(r.id)
                        }
                      >
                        <Trash2 size={15} />
                      </button>
                    </span>
                  </div>
                  {isOpen && (
                    <div className="lc-detail">
                      <dl className="lc-kv">
                        {r.description && (
                          <>
                            <dt>Description</dt>
                            <dd>{r.description}</dd>
                          </>
                        )}
                        <dt>Roles</dt>
                        <dd>
                          <Assignees
                            items={r.roles.map((x) => ({ key: x.id, label: x.name }))}
                            onRemove={(roleId, label) => window.confirm(`Unassign role "${label}" from "${r.name}"?`) && dropRole.mutate({ id: r.id, roleId })}
                          />
                        </dd>
                        <dt>Teams</dt>
                        <dd>
                          <Assignees
                            items={r.teams.map((x) => ({ key: x.handle, label: x.name }))}
                            onRemove={(handle, label) => window.confirm(`Unassign team "${label}" from "${r.name}"?`) && dropTeam.mutate({ id: r.id, handle })}
                          />
                        </dd>
                      </dl>
                    </div>
                  )}
                </div>
              );
            })}
            {queries.length === 0 && <p className="lc-empty">No restriction queries: everyone sees all logs.</p>}
            {queries.length > 0 && shown.length === 0 && !viewer && <p className="lc-empty">No restriction query matches.</p>}
          </>
        )}
      </section>
      {editing && (
        <RestrictionModal restriction={editing === 'new' ? null : editing} first={queries.length === 0} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), void refresh())} />
      )}
    </div>
  );
}

function Assignees({ items, onRemove }: { items: { key: string; label: string }[]; onRemove: (key: string, label: string) => void }) {
  if (items.length === 0) return <span className="muted">—</span>;
  return (
    <div className="da-assignees">
      {items.map((i) => (
        <span key={i.key} className="role-chip">
          {i.label}{' '}
          <button className="icon-plain" title={`Unassign ${i.label}`} onClick={() => onRemove(i.key, i.label)}>
            <X size={11} />
          </button>
        </span>
      ))}
    </div>
  );
}

function MatchCount({ query }: { query: string }) {
  const p = usePreview(query);
  if (p.isPending) return <span className="muted">…</span>;
  if (p.isError) return <span className="form-error">{(p.error as Error).message}</span>;
  return <span>{compact(p.data.count)}</span>;
}

function RestrictionModal({ restriction, first, onClose, onSaved }: { restriction: LogRestrictionQuery | null; first: boolean; onClose: () => void; onSaved: () => void }) {
  const me = useProfile();
  const adminRole = `${me.data?.org.id}-admin`;
  const roles = useRoles();
  const teams = useTeams(TEAMS_REQUEST);
  const [name, setName] = useState(restriction?.name ?? '');
  const [description, setDescription] = useState(restriction?.description ?? '');
  const [query, setQuery] = useState(restriction?.query ?? '');
  const [roleIds, setRoleIds] = useState<string[]>(restriction?.roles.map((r) => r.id) ?? []);
  const [teamHandles, setTeamHandles] = useState<string[]>(restriction?.teams.map((t) => t.handle) ?? []);
  const settled = useSettled(query.trim());
  const preview = usePreview(settled);
  const input: RestrictionQueryInput = { name, description, query, roleIds, teamHandles };
  const save = useMutation({ mutationFn: () => (restriction ? updateRestrictionQuery(restriction.id, input) : createRestrictionQuery(input)), onSuccess: onSaved });
  const toggle = (list: string[], set: (v: string[]) => void, value: string) => set(list.includes(value) ? list.filter((v) => v !== value) : [...list, value]);
  return (
    <Modal onClose={onClose} label={restriction ? 'Edit restriction query' : 'New restriction query'} width={720} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{restriction ? `Edit ${restriction.name}` : 'New Restriction Query'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Billing" maxLength={100} required autoFocus={!restriction} />
        </label>
        <label className="form-label">
          Description
          <input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Optional" maxLength={500} />
        </label>
        <label className="form-label">
          Query — assigned users see the logs matching this search
          <input className="mono" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="service:billing env:prod" required />
        </label>
        {settled !== '' && (
          <p className={`lm-estimate${preview.isError ? ' form-error' : ''}`}>
            {preview.isPending ? (
              <span className="muted">Counting matching logs…</span>
            ) : preview.isError ? (
              (preview.error as Error).message
            ) : (
              <span>
                <b>{compact(preview.data.count)}</b> logs matched in the last hour
              </span>
            )}
          </p>
        )}
        <div className="ix-excl-head">
          <b>Roles</b>
          <span className="muted">Every user with one of these roles. The admin role isn't listed: admins see every log.</span>
        </div>
        <div className="da-picker">
          {(roles.data ?? [])
            .filter((r) => r.id !== adminRole)
            .map((r) => (
              <label key={r.id}>
                <input type="checkbox" checked={roleIds.includes(r.id)} onChange={() => toggle(roleIds, setRoleIds, r.id)} />
                {r.name}
              </label>
            ))}
        </div>
        <div className="ix-excl-head">
          <b>Teams</b>
          <span className="muted">Direct members of these teams.</span>
        </div>
        <div className="da-picker">
          {(teams.data?.teams ?? []).map((t) => (
            <label key={t.handle}>
              <input type="checkbox" checked={teamHandles.includes(t.handle)} onChange={() => toggle(teamHandles, setTeamHandles, t.handle)} />
              {t.name}
            </label>
          ))}
        </div>
        {first && !restriction && (
          <p className="log-restricted-notice">
            <ShieldAlert size={14} /> This is the first restriction query: once it is saved, users who aren't admins and have no query assigned see no logs.
          </p>
        )}
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {restriction ? 'Save' : 'Create'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
