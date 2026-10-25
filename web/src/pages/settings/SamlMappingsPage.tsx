import { useMemo, useState } from 'react';
import { CheckCircle2, Plus, Search, X } from 'lucide-react';
import type { MappingKind } from '../../api/identity/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { useCreateMapping, useMappings, useRoles, useTeams } from './queries';

const GROUPS_CLAIM = 'http://schemas.microsoft.com/ws/2008/06/identity/claims/groups';

function dateAdded(ts: number): string {
  return new Date(ts).toLocaleString('en-US', { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }).replace(' AM', ' am').replace(' PM', ' pm');
}

export function SamlMappingsPage() {
  const [kind, setKind] = useState<MappingKind>('team');
  const [filter, setFilter] = useState('');
  const [adding, setAdding] = useState(false);
  const q = useMappings(kind);
  const rows = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return (q.data ?? []).filter((m) => !f || [m.targetName, m.attributeKey, m.attributeValue].some((v) => v.toLowerCase().includes(f)));
  }, [q.data, filter]);
  const noun = kind === 'team' ? 'Team' : 'Role';

  return (
    <div className="settings-pad">
      <h1 className="settings-title">SAML Group Mappings</h1>
      <nav className="settings-tabs">
        <button className={kind === 'team' ? 'active' : ''} onClick={() => setKind('team')}>
          Team Mappings
        </button>
        <button className={kind === 'role' ? 'active' : ''} onClick={() => setKind('role')}>
          Role Mappings
        </button>
      </nav>
      <section className="settings-card mapping-status">
        <CheckCircle2 size={24} className="ok-icon" />
        <b>{noun} Mappings enabled</b>
        <span className="muted">Users get the {kind === 'team' ? 'teams' : 'roles'} below when their identity provider sends a matching group claim at sign-in.</span>
      </section>
      <div className="mapping-bar">
        <label className="settings-search wide">
          <Search size={16} />
          <input placeholder={`Filter by ${kind} or IdP attributes`} value={filter} onChange={(e) => setFilter(e.target.value)} />
        </label>
        <button className="btn btn-primary" onClick={() => setAdding(true)}>
          <Plus size={14} /> New Mapping
        </button>
      </div>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <table className="settings-table bordered">
          <thead>
            <tr>
              <th>{noun.toUpperCase()}</th>
              <th>ATTRIBUTE KEY</th>
              <th>ATTRIBUTE VALUE</th>
              <th className="num">DATE ADDED</th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 ? (
              <tr>
                <td colSpan={4} className="muted center">
                  No mappings
                </td>
              </tr>
            ) : (
              rows.map((m) => (
                <tr key={m.id}>
                  <td>{m.targetName}</td>
                  <td className="ellipsis cell-key" title={m.attributeKey}>
                    {m.attributeKey}
                  </td>
                  <td className="mono-ish">{m.attributeValue}</td>
                  <td className="num">{dateAdded(m.createdAt)}</td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      )}
      {adding && <NewMapping kind={kind} onClose={() => setAdding(false)} />}
    </div>
  );
}

function NewMapping({ kind, onClose }: { kind: MappingKind; onClose: () => void }) {
  const teams = useTeams({ query: '', sort: 'name', offset: 0, limit: 200 });
  const roles = useRoles();
  const create = useCreateMapping();
  const [target, setTarget] = useState('');
  const [key, setKey] = useState(GROUPS_CLAIM);
  const [value, setValue] = useState('');
  const options = kind === 'team' ? (teams.data?.teams ?? []).map((t) => ({ id: t.handle, name: t.handle })) : (roles.data ?? []).map((r) => ({ id: r.id, name: r.name }));
  return (
    <Modal onClose={onClose} label="New mapping" width={560} height="auto">
      <form
        className="small-modal"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate({ kind, target, attributeKey: key, attributeValue: value }, { onSuccess: onClose });
        }}
      >
        <div className="modal-title-row">
          <h3>New {kind === 'team' ? 'Team' : 'Role'} Mapping</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          {kind === 'team' ? 'Team' : 'Role'}
          <select className="settings-input" value={target} onChange={(e) => setTarget(e.target.value)} required>
            <option value="">Select…</option>
            {options.map((o) => (
              <option key={o.id} value={o.id}>
                {o.name}
              </option>
            ))}
          </select>
        </label>
        <label className="form-label">
          Attribute key
          <input value={key} onChange={(e) => setKey(e.target.value)} required />
        </label>
        <label className="form-label">
          Attribute value
          <input value={value} onChange={(e) => setValue(e.target.value)} placeholder="Group object id or name" required />
        </label>
        {create.isError && <p className="form-error">{(create.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!target || !value.trim() || create.isPending}>
            Create
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function RolesPage() {
  const q = useRoles();
  return (
    <div className="settings-pad">
      <h1 className="settings-title">Roles</h1>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <table className="settings-table bordered">
          <thead>
            <tr>
              <th>NAME</th>
              <th>DESCRIPTION</th>
              <th className="num">USERS</th>
            </tr>
          </thead>
          <tbody>
            {q.data.map((r) => (
              <tr key={r.id}>
                <td>
                  {r.name} {r.managed && <span className="pill-chip small">Built-in</span>}
                </td>
                <td className="muted">{r.description}</td>
                <td className="num">{r.userCount}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
