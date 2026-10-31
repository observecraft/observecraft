import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Pencil, Plus, Trash2, X } from 'lucide-react';
import type { Suppression, SuppressionInput } from '../../api/security/client';
import { Modal } from '../../components/ui/Modal';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { useDeleteSuppression, useSaveSuppression, useSuppressions } from './queries';

/** Cloud SIEM → Settings: suppressions, and where the rules and packs are configured. */
export function SiemSettingsPage() {
  const q = useSuppressions();
  const del = useDeleteSuppression();
  const [editing, setEditing] = useState<Suppression | 'new' | null>(null);
  return (
    <div className="sec-page">
      <div className="sec-explorer-top">
        <h2 className="sec-h">Settings</h2>
      </div>
      <p className="muted">
        Turn log sources on and off in <Link to="/security/siem/content-packs">Content Packs</Link>, and change what triggers signals in <Link to="/security/siem/rules">Detection Rules</Link>.
      </p>
      <div className="sec-explorer-top">
        <h3>Suppressions</h3>
        <span className="syn-grow" />
        <button className="btn btn-primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> New Suppression
        </button>
      </div>
      <p className="muted">Silence expected signals (a deployment pipeline, a scanner you run) or leave some logs out of the rules.</p>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !q.data!.length ? (
        <Empty>
          <b>No suppressions.</b>
        </Empty>
      ) : (
        <div className="mon-table-wrap">
          <table className="mon-table syn-table">
            <thead>
              <tr>
                <th>NAME</th>
                <th>RULES</th>
                <th>SUPPRESSED WHEN</th>
                <th>EXCLUDED LOGS</th>
                <th>STATE</th>
                <th>UPDATED</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {q.data!.map((s) => (
                <tr key={s.id}>
                  <td>
                    <b>{s.name}</b>
                    {s.description && <div className="muted sec-sub">{s.description}</div>}
                  </td>
                  <td className="mono">{s.ruleQuery || <span className="muted">every rule</span>}</td>
                  <td className="mono">{s.suppressionQuery || '—'}</td>
                  <td className="mono">{s.dataExclusionQuery || '—'}</td>
                  <td>{!s.enabled ? 'Disabled' : s.expirationDate && s.expirationDate < Date.now() ? 'Expired' : s.startDate && s.startDate > Date.now() ? 'Scheduled' : 'Active'}</td>
                  <td className="nowrap muted">
                    {timeAgo(s.updatedAt)} by {s.updater}
                  </td>
                  <td className="nowrap">
                    <button className="btn btn-ghost" onClick={() => setEditing(s)} aria-label="Edit">
                      <Pencil size={14} />
                    </button>
                    <button className="btn btn-ghost" onClick={() => confirm(`Delete the suppression "${s.name}"?`) && del.mutate(s.id)} aria-label="Delete">
                      <Trash2 size={14} />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {editing && <SuppressionModal suppression={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

const dateInput = (ms?: number | null) => (ms ? new Date(ms - new Date().getTimezoneOffset() * 60_000).toISOString().slice(0, 16) : '');

function SuppressionModal({ suppression: s, onClose }: { suppression: Suppression | null; onClose: () => void }) {
  const save = useSaveSuppression(s?.id);
  const [f, setF] = useState<SuppressionInput>({
    name: s?.name ?? '',
    description: s?.description ?? '',
    enabled: s?.enabled ?? true,
    ruleQuery: s?.ruleQuery ?? '',
    suppressionQuery: s?.suppressionQuery ?? '',
    dataExclusionQuery: s?.dataExclusionQuery ?? '',
    startDate: s?.startDate ?? null,
    expirationDate: s?.expirationDate ?? null,
    tags: s?.tags ?? [],
  });
  const upd = (p: Partial<SuppressionInput>) => setF((x) => ({ ...x, ...p }));
  return (
    <Modal onClose={onClose} label={s ? 'Edit suppression' : 'New suppression'} width={620} height="auto">
      <form
        className="sec-modal"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate(f, { onSuccess: onClose });
        }}
      >
        <header>
          <h3>{s ? 'Edit suppression' : 'New suppression'}</h3>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <label>
          Name
          <input autoFocus required value={f.name} onChange={(e) => upd({ name: e.target.value })} />
        </label>
        <label>
          Description
          <input value={f.description ?? ''} onChange={(e) => upd({ description: e.target.value })} />
        </label>
        <label>
          Rules (rule query; empty: every rule)
          <input className="mono" placeholder="source:cloudtrail tactic:TA0006*" value={f.ruleQuery ?? ''} onChange={(e) => upd({ ruleQuery: e.target.value })} />
        </label>
        <label>
          Suppress signals where
          <input className="mono" placeholder="@userIdentity.arn:*ci-deployer env:staging" value={f.suppressionQuery ?? ''} onChange={(e) => upd({ suppressionQuery: e.target.value })} />
        </label>
        <label>
          Leave out logs matching (one term)
          <input className="mono" placeholder="@network.client.ip:192.0.2.10" value={f.dataExclusionQuery ?? ''} onChange={(e) => upd({ dataExclusionQuery: e.target.value })} />
        </label>
        <div className="sec-row">
          <label>
            Starts
            <input type="datetime-local" value={dateInput(f.startDate)} onChange={(e) => upd({ startDate: e.target.value ? new Date(e.target.value).getTime() : null })} />
          </label>
          <label>
            Expires
            <input type="datetime-local" value={dateInput(f.expirationDate)} onChange={(e) => upd({ expirationDate: e.target.value ? new Date(e.target.value).getTime() : null })} />
          </label>
        </div>
        <label className="sec-check-label">
          <input type="checkbox" checked={f.enabled ?? true} onChange={(e) => upd({ enabled: e.target.checked })} /> Enabled
        </label>
        {save.isError && <p className="sec-error">{(save.error as Error).message}</p>}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending || !f.name.trim()}>
            {s ? 'Save' : 'Create'}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
