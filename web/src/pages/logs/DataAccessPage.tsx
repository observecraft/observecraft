import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Pencil, Plus, ShieldAlert, Trash2, X } from 'lucide-react';
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
import { useProfile, useRoles, useTeams } from '../settings/queries';

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
  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Data Access</h1>
        <button className="btn btn-primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> New Restriction Query
        </button>
      </div>
      <p className="muted">
        Restriction queries are log searches assigned to <Link to="/organization-settings/roles">roles</Link> and <Link to="/teams">teams</Link>. They decide which logs their users can search, chart,
        tail and export.
      </p>
      <Rules />
      {queries.length > 0 && (
        <p className="log-restricted-notice">
          <ShieldAlert size={14} /> Restrictions are on: users who aren't admins and have no query assigned see no logs.
        </p>
      )}
      {error && <p className="form-error">{(error as Error).message}</p>}
      {queries.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No restriction queries: everyone sees all logs.</p>
        </section>
      ) : (
        <table className="settings-table ix-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>QUERY</th>
              <th>ROLES</th>
              <th>TEAMS</th>
              <th>LOGS (LAST HOUR)</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {queries.map((r) => (
              <tr key={r.id}>
                <td>
                  <b>{r.name}</b>
                  {r.description && <div className="muted">{r.description}</div>}
                </td>
                <td className="mono">
                  <Link to={`/logs?query=${encodeURIComponent(r.query)}`}>{r.query}</Link>
                </td>
                <td>
                  <Assignees
                    items={r.roles.map((x) => ({ key: x.id, label: x.name }))}
                    onRemove={(roleId, label) => window.confirm(`Unassign role "${label}" from "${r.name}"?`) && dropRole.mutate({ id: r.id, roleId })}
                  />
                </td>
                <td>
                  <Assignees
                    items={r.teams.map((x) => ({ key: x.handle, label: x.name }))}
                    onRemove={(handle, label) => window.confirm(`Unassign team "${label}" from "${r.name}"?`) && dropTeam.mutate({ id: r.id, handle })}
                  />
                </td>
                <td>
                  <MatchCount query={r.query} />
                </td>
                <td className="ix-actions">
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
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
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
