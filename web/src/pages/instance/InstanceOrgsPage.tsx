import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Building2, Plus, X } from 'lucide-react';
import { createOrg, deleteOrg, instanceAudit, listOrgs, updateOrg, updateOrgLimits, type InstanceOrg } from '../../api/instance/client';
import { dataSettings, type MetricRetention } from '../../api/logIndexes/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { compact, timeAgo } from '../../lib/format';
import { LinkModal } from '../settings/UsersPage';
import { useProfile } from '../settings/queries';
import '../settings/settings.css';

const KEY = ['instance', 'orgs'];

/** Instance Admin → Organizations: create, rename, suspend and delete the install's organizations. */
export function InstanceOrgsPage() {
  const me = useProfile();
  const qc = useQueryClient();
  const orgs = useQuery({ queryKey: KEY, queryFn: listOrgs, enabled: !!me.data?.isInstanceAdmin });
  const audit = useQuery({ queryKey: ['instance', 'audit'], queryFn: instanceAudit, enabled: !!me.data?.isInstanceAdmin });
  const [creating, setCreating] = useState(false);
  const [invite, setInvite] = useState<{ org: string; url: string } | null>(null);
  const [limits, setLimits] = useState<InstanceOrg | null>(null);
  const refresh = () => void qc.invalidateQueries({ queryKey: ['instance'] });
  const update = useMutation({ mutationFn: (v: { id: string; status?: 'active' | 'suspended'; name?: string }) => updateOrg(v.id, v), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteOrg, onSuccess: refresh });

  if (me.isPending) return <Loading label="Loading…" />;
  if (!me.data?.isInstanceAdmin)
    return (
      <div className="settings-pad">
        <h1 className="settings-title">Instance Admin</h1>
        <p className="muted">Only instance administrators can manage organizations.</p>
      </div>
    );

  const onDelete = (o: InstanceOrg) => {
    const typed = window.prompt(`Permanently delete "${o.name}" with all its users, dashboards, monitors, incidents and telemetry?\n\nType the organization id (${o.id}) to confirm.`);
    if (typed === o.id) remove.mutate(o.id);
  };
  const onRename = (o: InstanceOrg) => {
    const name = window.prompt('Organization name', o.name)?.trim();
    if (name && name !== o.name) update.mutate({ id: o.id, name });
  };

  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">
          <Building2 size={22} /> Organizations
        </h1>
        <button className="btn btn-primary" onClick={() => setCreating(true)}>
          <Plus size={14} /> New Organization
        </button>
      </div>
      <p className="muted">Each organization has its own users, roles, SSO, dashboards, monitors, incidents and telemetry. People can belong to several and switch from the account menu.</p>
      {(update.error || remove.error) && <p className="form-error">{((update.error ?? remove.error) as Error).message}</p>}
      {orgs.isPending ? (
        <Loading label="Loading organizations…" />
      ) : orgs.isError ? (
        <ErrorState error={orgs.error} onRetry={() => orgs.refetch()} />
      ) : (
        <table className="settings-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>ID</th>
              <th>STATUS</th>
              <th>MEMBERS</th>
              <th>ADMINS</th>
              <th>LOGS (24H)</th>
              <th>SPANS (24H)</th>
              <th>LIMITS</th>
              <th>CREATED</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {orgs.data.map((o) => (
              <tr key={o.id} className={o.status === 'suspended' ? 'row-muted' : ''}>
                <td>
                  <b>{o.name}</b>
                  {o.primary && <span className="pill-chip small">Primary</span>}
                </td>
                <td>
                  <code>{o.id}</code>
                </td>
                <td>{o.status === 'suspended' ? <span className="pill-chip small">Suspended</span> : 'Active'}</td>
                <td>{o.members}</td>
                <td className="ellipsis" title={o.admins.join(', ')}>
                  {o.admins.slice(0, 2).join(', ')}
                  {o.admins.length > 2 && <span className="more-chip">+{o.admins.length - 2}</span>}
                </td>
                <td>{compact(o.logs24h)}</td>
                <td>{compact(o.spans24h)}</td>
                <td className="muted small" title="Longest log, span and metric retention (full resolution / 5-minute / 1-hour rollups) and number of log indexes the org may configure">
                  logs ≤ {o.maxLogRetentionDays} d · spans ≤ {o.maxSpanRetentionDays} d · metrics ≤ {o.maxMetrics.raw}/{o.maxMetrics.rollup5m}/{o.maxMetrics.rollup1h} d · {o.maxIndexes} indexes
                </td>
                <td>{timeAgo(o.createdAt)}</td>
                <td className="col-actions">
                  <Menu
                    trigger={<span aria-label="Organization actions">⋯</span>}
                    triggerClassName="icon-plain"
                    items={[
                      { label: 'Rename', onClick: () => onRename(o) },
                      { label: 'Retention limits…', onClick: () => setLimits(o) },
                      ...(o.primary
                        ? []
                        : [
                            o.status === 'active'
                              ? {
                                  label: 'Suspend',
                                  onClick: () => window.confirm(`Suspend ${o.name}? Its members are signed out and its keys stop working.`) && update.mutate({ id: o.id, status: 'suspended' }),
                                }
                              : { label: 'Reactivate', onClick: () => update.mutate({ id: o.id, status: 'active' }) },
                            { label: 'Delete…', onClick: () => onDelete(o) },
                          ]),
                    ]}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <h2 className="settings-subtitle">Recent instance activity</h2>
      {audit.data && audit.data.length === 0 && <p className="muted">No organization changes yet.</p>}
      {audit.data && audit.data.length > 0 && (
        <table className="settings-table">
          <thead>
            <tr>
              <th>WHEN</th>
              <th>WHO</th>
              <th>ACTION</th>
              <th>ORGANIZATION</th>
              <th>DETAIL</th>
            </tr>
          </thead>
          <tbody>
            {audit.data.slice(0, 20).map((e) => (
              <tr key={e.id}>
                <td>{timeAgo(e.createdAt)}</td>
                <td>{e.actor}</td>
                <td>
                  <code>{e.action}</code>
                </td>
                <td>{e.target}</td>
                <td className="muted">{e.detail}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {creating && (
        <CreateOrgModal
          onClose={() => setCreating(false)}
          onCreated={(org, url) => {
            setCreating(false);
            setInvite({ org, url });
            refresh();
          }}
        />
      )}
      {limits && <LimitsModal org={limits} onClose={() => setLimits(null)} onSaved={() => (setLimits(null), refresh())} />}
      {invite && (
        <LinkModal
          title={`${invite.org} created`}
          links={[{ label: 'First admin', url: invite.url }]}
          note="Send this link to the organization's first admin (valid 72 hours, single use). If they already have an ObserveCraft account, they join with their current password."
          onClose={() => setInvite(null)}
        />
      )}
    </div>
  );
}

const slug = (s: string) =>
  s
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40);

function CreateOrgModal({ onClose, onCreated }: { onClose: () => void; onCreated: (org: string, url: string) => void }) {
  const [name, setName] = useState('');
  const [id, setId] = useState('');
  const [idTouched, setIdTouched] = useState(false);
  const [email, setEmail] = useState('');
  const create = useMutation({ mutationFn: () => createOrg(idTouched ? id : slug(name), name.trim(), email.trim()), onSuccess: (r) => onCreated(name.trim(), r.inviteUrl) });
  const shownId = idTouched ? id : slug(name);
  return (
    <Modal onClose={onClose} label="New organization" width={520} height="auto">
      <form
        className="small-modal"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>New Organization</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name
          <input className="settings-input" value={name} onChange={(e) => setName(e.target.value)} maxLength={120} placeholder="Acme Retail" required autoFocus />
        </label>
        <label className="form-label">
          ID
          <input
            className="settings-input"
            value={shownId}
            onChange={(e) => {
              setIdTouched(true);
              setId(e.target.value.toLowerCase());
            }}
            pattern="[a-z0-9]([a-z0-9-]*[a-z0-9])?"
            minLength={2}
            maxLength={40}
            placeholder="acme-retail"
            required
          />
          <span className="muted small">Used in URLs and SSO endpoints; can't be changed later.</span>
        </label>
        <label className="form-label">
          First admin email
          <input className="settings-input" type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="admin@example.com" required />
        </label>
        {create.error && <p className="form-error">{(create.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={create.isPending}>
            Create Organization
          </button>
        </div>
      </form>
    </Modal>
  );
}

function LimitsModal({ org, onClose, onSaved }: { org: InstanceOrg; onClose: () => void; onSaved: () => void }) {
  const [logs, setLogs] = useState(org.maxLogRetentionDays);
  const [spans, setSpans] = useState(org.maxSpanRetentionDays);
  const [indexes, setIndexes] = useState(org.maxIndexes);
  const [metrics, setMetrics] = useState<MetricRetention>(org.maxMetrics);
  // The metric retention tiers (the same for every org).
  const tiers = useQuery({ queryKey: ['data-settings'], queryFn: dataSettings }).data?.metricRetentionTiers ?? [];
  const save = useMutation({
    mutationFn: () => updateOrgLimits(org.id, { maxLogRetentionDays: logs, maxSpanRetentionDays: spans, maxIndexes: indexes, maxMetrics: metrics }),
    onSuccess: onSaved,
  });
  return (
    <Modal onClose={onClose} label="Retention limits" width={520} height="auto">
      <form
        className="small-modal"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>Retention limits — {org.name}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <p className="muted">The most this organization's admins can configure. Lowering a limit also lowers settings above it (for logs indexed from now on).</p>
        <label className="form-label">
          Longest log index retention (days)
          <input type="number" min={1} value={logs} onChange={(e) => setLogs(Number(e.target.value))} required />
        </label>
        <label className="form-label">
          Longest span retention (days)
          <input type="number" min={1} value={spans} onChange={(e) => setSpans(Number(e.target.value))} required />
        </label>
        <label className="form-label">
          Log indexes
          <input type="number" min={1} max={1000} value={indexes} onChange={(e) => setIndexes(Number(e.target.value))} required />
        </label>
        {(
          [
            ['raw', 'Longest metric retention at full resolution (days)'],
            ['rollup5m', 'Longest metric retention as 5-minute rollups (days)'],
            ['rollup1h', 'Longest metric retention as 1-hour rollups (days)'],
          ] as const
        ).map(([key, label]) => (
          <label key={key} className="form-label">
            {label}
            <select value={metrics[key]} onChange={(e) => setMetrics({ ...metrics, [key]: Number(e.target.value) })}>
              {[...new Set([...tiers, metrics[key]])]
                .sort((a, b) => a - b)
                .map((x) => (
                  <option key={x} value={x}>
                    {x}
                  </option>
                ))}
            </select>
          </label>
        ))}
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            Save Limits
          </button>
        </div>
      </form>
    </Modal>
  );
}
