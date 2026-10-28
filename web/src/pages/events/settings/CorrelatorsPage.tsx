import { useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Pencil, Plus, Trash2, X } from 'lucide-react';
import { createCorrelator, deleteCorrelator, eventFields, listCorrelators, updateCorrelator, workItems, type EventCorrelator, type EventCorrelatorInput } from '../../../api/events/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { timeAgo } from '../../../lib/format';
import { useAccess } from '../../settings/queries';

export const CORRELATORS_KEY = ['events', 'correlators'];
const WINDOWS = [5, 10, 15, 30, 60, 120, 240, 720, 1440];
const CORE_KEYS = ['source', 'host', 'status', 'title'];
const HOUR_MS = 3_600_000;

export const windowLabel = (m: number) => (m < 60 ? `${m} min` : m === 1440 ? '24 hours' : `${m / 60} hour${m === 60 ? '' : 's'}`);

/** Event Management → Define Correlators: rules that group related events into one work item. */
export function CorrelatorsPage() {
  const qc = useQueryClient();
  const { canWrite } = useAccess();
  const [params, setParams] = useSearchParams();
  const q = useQuery({ queryKey: CORRELATORS_KEY, queryFn: listCorrelators });
  const [editing, setEditing] = useState<EventCorrelator | 'new' | null>(params.get('new') ? 'new' : null);
  const refresh = () => qc.invalidateQueries({ queryKey: ['events'] });
  const remove = useMutation({ mutationFn: deleteCorrelator, onSuccess: refresh });
  const toggle = useMutation({ mutationFn: (c: EventCorrelator) => updateCorrelator(c.id, { ...c, enabled: !c.enabled }), onSuccess: refresh });
  const close = () => {
    setEditing(null);
    if (params.get('new')) setParams({}, { replace: true });
  };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Define Correlators</h1>
        {canWrite && (
          <button className="btn btn-primary" onClick={() => setEditing('new')}>
            <Plus size={14} /> New Correlator
          </button>
        )}
      </div>
      <p className="muted">
        A correlator groups the warning and error events its query matches into one work item per distinct group-by values, for as long as they keep arriving within its time window. The first matching
        correlator, in order, takes an event; events no correlator matches stay grouped per affected object in the <Link to="/event/inbox">Triage Inbox</Link>.
      </p>
      {(remove.error || toggle.error) && <p className="form-error">{((remove.error ?? toggle.error) as Error).message}</p>}
      {q.data.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No correlators yet: work items are grouped per affected object.</p>
        </section>
      ) : (
        <table className="settings-table ix-table">
          <thead>
            <tr>
              <th>ORDER</th>
              <th>NAME</th>
              <th>QUERY</th>
              <th>GROUP BY</th>
              <th>TIME WINDOW</th>
              <th>ENABLED</th>
              <th>LAST UPDATED</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {q.data.map((c) => (
              <tr key={c.id}>
                <td className="muted">{c.position}</td>
                <td>{c.name}</td>
                <td className="mono">{c.query || <span className="muted">all events</span>}</td>
                <td>
                  {c.groupBy.length === 0 ? (
                    <span className="muted">—</span>
                  ) : (
                    c.groupBy.map((k) => (
                      <code key={k} className="lm-tag">
                        {k}
                      </code>
                    ))
                  )}
                </td>
                <td>{windowLabel(c.windowMinutes)}</td>
                <td>
                  <button
                    role="switch"
                    aria-checked={c.enabled}
                    aria-label={c.enabled ? `Disable ${c.name}` : `Enable ${c.name}`}
                    className={`switch${c.enabled ? ' on' : ''}`}
                    disabled={!canWrite || toggle.isPending}
                    onClick={() => toggle.mutate(c)}
                  />
                </td>
                <td className="muted" title={c.updatedBy ? `by ${c.updatedBy}` : undefined}>
                  {timeAgo(c.updatedAt)}
                  {c.updatedBy && ` · ${c.updatedBy}`}
                </td>
                <td className="ix-actions">
                  {canWrite && (
                    <>
                      <button className="icon-plain" title="Edit" onClick={() => setEditing(c)}>
                        <Pencil size={15} />
                      </button>
                      <button className="icon-plain" title="Delete" onClick={() => window.confirm(`Delete correlator "${c.name}"? Its events go back to the default grouping.`) && remove.mutate(c.id)}>
                        <Trash2 size={15} />
                      </button>
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {editing && <CorrelatorModal correlator={editing === 'new' ? null : editing} next={q.data.length} onClose={close} onSaved={() => (close(), void refresh())} />}
    </div>
  );
}

function CorrelatorModal({ correlator, next, onClose, onSaved }: { correlator: EventCorrelator | null; next: number; onClose: () => void; onSaved: () => void }) {
  const [name, setName] = useState(correlator?.name ?? '');
  const [query, setQuery] = useState(correlator?.query ?? '');
  const [groupBy, setGroupBy] = useState(correlator?.groupBy.join(', ') ?? '');
  const [windowMinutes, setWindowMinutes] = useState(correlator?.windowMinutes ?? 30);
  const [position, setPosition] = useState(correlator?.position ?? next);
  const [enabled, setEnabled] = useState(correlator?.enabled ?? true);
  const keys = useQuery({
    queryKey: ['events', 'correlator-fields'],
    queryFn: () => {
      const to = Date.now();
      return eventFields({ range: { from: to - 24 * HOUR_MS, to } });
    },
    staleTime: 60_000,
  });
  // What the query matches now: the inbox's work items over the last day.
  const preview = useQuery({
    queryKey: ['events', 'correlator-preview', query],
    queryFn: () => {
      const to = Date.now();
      return workItems({ range: { from: to - 24 * HOUR_MS, to }, query });
    },
    staleTime: 30_000,
  });
  const input: EventCorrelatorInput = {
    name,
    query,
    groupBy: groupBy
      .split(',')
      .map((k) => k.trim())
      .filter(Boolean),
    windowMinutes,
    enabled,
    position,
  };
  const save = useMutation({ mutationFn: () => (correlator ? updateCorrelator(correlator.id, input) : createCorrelator(input)), onSuccess: onSaved });
  const events = preview.data?.items.reduce((n, i) => n + i.count, 0);
  return (
    <Modal onClose={onClose} label={correlator ? 'Edit correlator' : 'New correlator'} width={720} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{correlator ? `Edit ${correlator.name}` : 'New Correlator'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Checkout pods restarting" maxLength={200} required autoFocus />
        </label>
        <label className="form-label">
          Query — warning and error events matching this search are correlated
          <input className="mono" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="all events (or e.g. source:kubernetes kube_namespace:shop)" />
        </label>
        <p className="muted es-preview">
          {preview.isPending ? 'Counting matching events…' : preview.isError ? (preview.error as Error).message : `${(events ?? 0).toLocaleString()} warning and error events matched in the past day.`}
        </p>
        <label className="form-label">
          Group by — one work item per distinct values (comma separated)
          <input className="mono" list="es-group-keys" value={groupBy} onChange={(e) => setGroupBy(e.target.value)} placeholder="e.g. kube_namespace, service" />
          <datalist id="es-group-keys">
            {[...CORE_KEYS, ...(keys.data ?? []).map((k) => k.key)].map((k) => (
              <option key={k} value={k} />
            ))}
          </datalist>
        </label>
        <div className="ar-grid">
          <label className="form-label">
            Time window — events further apart start a new work item
            <select value={windowMinutes} onChange={(e) => setWindowMinutes(Number(e.target.value))}>
              {WINDOWS.map((w) => (
                <option key={w} value={w}>
                  {windowLabel(w)}
                </option>
              ))}
            </select>
          </label>
          <label className="form-label">
            Order — lower runs first
            <input type="number" min={0} value={position} onChange={(e) => setPosition(Number(e.target.value))} />
          </label>
        </div>
        <label className="ar-check">
          <input type="checkbox" className="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Enabled
        </label>
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {correlator ? 'Save' : 'Create Correlator'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
