import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Plus, Trash2, X } from 'lucide-react';
import { createMaintenanceWindow, deleteMaintenanceWindow, listMaintenanceWindows } from '../../../api/events/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { formatLongDate } from '../../../lib/format';
import { useAccess } from '../../settings/queries';

const KEY = ['events', 'maintenance-windows'];
const HOUR_MS = 3_600_000;

/** Scheduled, active or ended, at `now`. */
export function windowState(w: { startsAt: number; endsAt: number }, now = Date.now()): 'scheduled' | 'active' | 'ended' {
  return now < w.startsAt ? 'scheduled' : now <= w.endsAt ? 'active' : 'ended';
}

/** `<input type="datetime-local">` value for an epoch ms, in local time. */
const localInput = (ms: number) => {
  const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
  return d.toISOString().slice(0, 16);
};

/** Event Management → Maintenance Windows: expected events (deploys, upgrades) that shouldn't page anyone. */
export function MaintenanceWindowsPage() {
  const qc = useQueryClient();
  const { canWrite } = useAccess();
  const q = useQuery({ queryKey: KEY, queryFn: listMaintenanceWindows });
  const [creating, setCreating] = useState(false);
  const refresh = () => qc.invalidateQueries({ queryKey: ['events'] });
  const remove = useMutation({ mutationFn: deleteMaintenanceWindow, onSuccess: refresh });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Maintenance Windows</h1>
        {canWrite && (
          <button className="btn btn-primary" onClick={() => setCreating(true)}>
            <Plus size={14} /> New Maintenance Window
          </button>
        )}
      </div>
      <p className="muted">
        Events matching a window's scope while it runs are expected. Work items made only of such events are suppressed in the <Link to="/event/inbox">Triage Inbox</Link>: hidden from Open and Closed,
        listed under Suppressed.
      </p>
      {remove.error && <p className="form-error">{(remove.error as Error).message}</p>}
      {q.data.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No maintenance windows.</p>
        </section>
      ) : (
        <table className="settings-table ix-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>SCOPE</th>
              <th>STARTS</th>
              <th>ENDS</th>
              <th>STATUS</th>
              <th>CREATED BY</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {q.data.map((w) => {
              const state = windowState(w);
              return (
                <tr key={w.id}>
                  <td>{w.name}</td>
                  <td className="mono">{w.query || <span className="muted">all events</span>}</td>
                  <td>{formatLongDate(w.startsAt)}</td>
                  <td>{formatLongDate(w.endsAt)}</td>
                  <td>
                    <span className={`es-window es-window-${state}`}>{state === 'active' ? 'Active' : state === 'scheduled' ? 'Scheduled' : 'Ended'}</span>
                  </td>
                  <td className="muted">{w.createdBy || '—'}</td>
                  <td className="ix-actions">
                    {canWrite && (
                      <button className="icon-plain" title="Delete" onClick={() => window.confirm(`Delete maintenance window "${w.name}"?`) && remove.mutate(w.id)}>
                        <Trash2 size={15} />
                      </button>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {creating && <WindowModal onClose={() => setCreating(false)} onSaved={() => (setCreating(false), void refresh())} />}
    </div>
  );
}

function WindowModal({ onClose, onSaved }: { onClose: () => void; onSaved: () => void }) {
  const [now] = useState(() => Math.floor(Date.now() / 60_000) * 60_000);
  const [name, setName] = useState('');
  const [query, setQuery] = useState('');
  const [startsAt, setStartsAt] = useState(localInput(now));
  const [endsAt, setEndsAt] = useState(localInput(now + HOUR_MS));
  const save = useMutation({
    mutationFn: () => createMaintenanceWindow({ name, query, startsAt: new Date(startsAt).getTime(), endsAt: new Date(endsAt).getTime() }),
    onSuccess: onSaved,
  });
  return (
    <Modal onClose={onClose} label="New maintenance window" width={640} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>New Maintenance Window</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Billing database upgrade" maxLength={200} required autoFocus />
        </label>
        <label className="form-label">
          Scope — events matching this search are expected
          <input className="mono" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="all events (or e.g. service:billing env:prod)" />
        </label>
        <div className="ar-grid">
          <label className="form-label">
            Starts
            <input type="datetime-local" value={startsAt} onChange={(e) => setStartsAt(e.target.value)} required />
          </label>
          <label className="form-label">
            Ends
            <input type="datetime-local" value={endsAt} onChange={(e) => setEndsAt(e.target.value)} required />
          </label>
        </div>
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            Create Window
          </button>
        </div>
      </form>
    </Modal>
  );
}
