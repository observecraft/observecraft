import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowDown, ArrowUp, Pencil, Plus, Trash2, X } from 'lucide-react';
import { createIndex, deleteIndex, listIndexes, reorderIndexes, updateIndex, type ExclusionFilter, type LogIndex, type LogIndexInput } from '../../api/logIndexes/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { compact } from '../../lib/format';
import { useProfile } from '../settings/queries';

const RETENTIONS = [1, 3, 7, 15, 30, 45, 60, 90, 180, 365, 730];
const KEY = ['log-indexes'];

/** Log Configuration → Indexes: route logs to indexes with their own retention, quota and exclusions. */
export function IndexesPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  const q = useQuery({ queryKey: KEY, queryFn: listIndexes, refetchInterval: 30_000 });
  const [editing, setEditing] = useState<LogIndex | 'new' | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: KEY });
  const reorder = useMutation({ mutationFn: reorderIndexes, onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteIndex, onSuccess: refresh });

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const { indexes, ingestedToday, maxRetentionDays, maxIndexes } = q.data;
  const indexedToday = indexes.reduce((n, i) => n + i.indexedToday, 0);
  const move = (i: number, d: -1 | 1) => {
    const ids = indexes.map((x) => x.id);
    [ids[i], ids[i + d]] = [ids[i + d], ids[i]];
    reorder.mutate(ids);
  };

  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Indexes</h1>
        {isAdmin && (
          <button className="btn btn-primary" onClick={() => setEditing('new')} disabled={indexes.length >= maxIndexes}>
            <Plus size={14} /> New Index
          </button>
        )}
      </div>
      <p className="muted">
        Each incoming log goes to the <b>first</b> index whose filter matches; the index decides how long it stays searchable. Logs that no index keeps (no match, excluded, or over quota) still reach{' '}
        <Link to="/logs/livetail">Live Tail</Link> and <Link to="/logs/archives">archives</Link>. Search one index with <code>index:&lt;name&gt;</code>.
      </p>
      <div className="ix-summary">
        <span>
          <b>{compact(ingestedToday)}</b> logs received today
        </span>
        <span>
          <b>{compact(indexedToday)}</b> indexed
        </span>
        <span className="muted">
          Up to {maxRetentionDays} days retention and {maxIndexes} indexes for this organization
        </span>
      </div>
      {(reorder.error || remove.error) && <p className="form-error">{((reorder.error ?? remove.error) as Error).message}</p>}
      {indexes.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No indexes: logs are only available in Live Tail and archives.</p>
        </section>
      ) : (
        <table className="settings-table ix-table">
          <thead>
            <tr>
              <th>#</th>
              <th>NAME</th>
              <th>FILTER</th>
              <th>RETENTION</th>
              <th>EXCLUSION FILTERS</th>
              <th>INDEXED TODAY</th>
              {isAdmin && <th />}
            </tr>
          </thead>
          <tbody>
            {indexes.map((ix, i) => {
              const pct = ix.dailyQuota ? Math.min(100, (ix.indexedToday / ix.dailyQuota) * 100) : null;
              const active = ix.exclusionFilters.filter((e) => e.enabled);
              return (
                <tr key={ix.id}>
                  <td className="muted">{i + 1}</td>
                  <td>
                    <Link className="mono" to={`/logs?query=${encodeURIComponent(`index:${ix.name}`)}`}>
                      {ix.name}
                    </Link>
                  </td>
                  <td className="mono">{ix.filter || <span className="muted">all logs</span>}</td>
                  <td>{ix.retentionDays} days</td>
                  <td>
                    {active.length === 0 ? (
                      <span className="muted">—</span>
                    ) : (
                      active.map((e) => (
                        <div key={e.name + e.query} className="ix-excl">
                          <code>{e.query || '*'}</code> {e.sampleRate}%
                        </div>
                      ))
                    )}
                  </td>
                  <td>
                    {ix.indexedToday.toLocaleString()}
                    {ix.dailyQuota && (
                      <div className="ix-quota" title={`Daily quota ${ix.dailyQuota.toLocaleString()} (resets ${String(ix.quotaResetHour).padStart(2, '0')}:00 UTC)`}>
                        <span className={`ix-quota-bar${pct! >= 100 ? ' full' : pct! >= 80 ? ' high' : ''}`}>
                          <i style={{ width: `${pct}%` }} />
                        </span>
                        <span className="muted">{pct! >= 100 ? 'quota reached' : `of ${compact(ix.dailyQuota)}`}</span>
                      </div>
                    )}
                  </td>
                  {isAdmin && (
                    <td className="ix-actions">
                      <button className="icon-plain" title="Move up" disabled={i === 0 || reorder.isPending} onClick={() => move(i, -1)}>
                        <ArrowUp size={15} />
                      </button>
                      <button className="icon-plain" title="Move down" disabled={i === indexes.length - 1 || reorder.isPending} onClick={() => move(i, 1)}>
                        <ArrowDown size={15} />
                      </button>
                      <button className="icon-plain" title="Edit" onClick={() => setEditing(ix)}>
                        <Pencil size={15} />
                      </button>
                      <button
                        className="icon-plain"
                        title="Delete"
                        onClick={() => window.confirm(`Delete index "${ix.name}"? New logs stop going there; logs already in it stay searchable until they expire.`) && remove.mutate(ix.id)}
                      >
                        <Trash2 size={15} />
                      </button>
                    </td>
                  )}
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {editing && <IndexModal index={editing === 'new' ? null : editing} maxRetention={maxRetentionDays} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), void refresh())} />}
    </div>
  );
}

function IndexModal({ index, maxRetention, onClose, onSaved }: { index: LogIndex | null; maxRetention: number; onClose: () => void; onSaved: () => void }) {
  const [name, setName] = useState(index?.name ?? '');
  const [filter, setFilter] = useState(index?.filter ?? '');
  const [retention, setRetention] = useState(index?.retentionDays ?? Math.min(15, maxRetention));
  const [quota, setQuota] = useState(index?.dailyQuota ? String(index.dailyQuota) : '');
  const [resetHour, setResetHour] = useState(index?.quotaResetHour ?? 0);
  const [exclusions, setExclusions] = useState<ExclusionFilter[]>(index?.exclusionFilters ?? []);
  const input: LogIndexInput = { name, filter, retentionDays: retention, dailyQuota: quota.trim() ? Number(quota) : null, quotaResetHour: resetHour, exclusionFilters: exclusions };
  const save = useMutation({ mutationFn: () => (index ? updateIndex(index.id, input) : createIndex(input)), onSuccess: onSaved });
  const setEx = (i: number, p: Partial<ExclusionFilter>) => setExclusions((xs) => xs.map((x, j) => (j === i ? { ...x, ...p } : x)));
  const options = RETENTIONS.filter((d) => d <= maxRetention);
  if (!options.includes(retention)) options.push(retention);
  return (
    <Modal onClose={onClose} label={index ? 'Edit index' : 'New index'} width={680} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{index ? `Edit ${index.name}` : 'New Index'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name
          <input value={name} onChange={(e) => setName(e.target.value.toLowerCase())} placeholder="errors" maxLength={64} required autoFocus={!index} />
        </label>
        <label className="form-label">
          Filter — logs matching this search go here (if no earlier index took them)
          <input className="mono" value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="all logs (or e.g. env:prod status:(error OR warn))" />
        </label>
        <div className="ar-grid">
          <label className="form-label">
            Retention
            <select value={retention} onChange={(e) => setRetention(Number(e.target.value))}>
              {options
                .sort((a, b) => a - b)
                .map((d) => (
                  <option key={d} value={d}>
                    {d} day{d === 1 ? '' : 's'}
                  </option>
                ))}
            </select>
          </label>
          <label className="form-label">
            Daily quota (logs)
            <input type="number" min={1} value={quota} onChange={(e) => setQuota(e.target.value)} placeholder="unlimited" />
          </label>
          <label className="form-label">
            Quota resets at (UTC)
            <select value={resetHour} onChange={(e) => setResetHour(Number(e.target.value))}>
              {Array.from({ length: 24 }, (_, h) => (
                <option key={h} value={h}>
                  {String(h).padStart(2, '0')}:00
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="ix-excl-head">
          <b>Exclusion filters</b>
          <span className="muted">Drop part of the matching logs from this index (first matching filter applies). They still reach Live Tail and archives.</span>
        </div>
        {exclusions.map((x, i) => (
          <div key={i} className="ix-excl-row">
            <input type="checkbox" checked={x.enabled} onChange={(e) => setEx(i, { enabled: e.target.checked })} title="Enabled" />
            <input value={x.name} onChange={(e) => setEx(i, { name: e.target.value })} placeholder="Name" className="ix-excl-name" />
            <input value={x.query} onChange={(e) => setEx(i, { query: e.target.value })} placeholder="status:debug" className="mono ix-excl-query" />
            <span className="ix-excl-rate">
              exclude
              <input type="number" min={0} max={100} value={x.sampleRate} onChange={(e) => setEx(i, { sampleRate: Number(e.target.value) })} />%
            </span>
            <button type="button" className="icon-plain" title="Remove" onClick={() => setExclusions((xs) => xs.filter((_, j) => j !== i))}>
              <Trash2 size={15} />
            </button>
          </div>
        ))}
        <button type="button" className="btn btn-sm ix-add-excl" onClick={() => setExclusions((xs) => [...xs, { name: '', query: '', sampleRate: 100, enabled: true }])}>
          <Plus size={13} /> Add Exclusion Filter
        </button>
        {index && index.retentionDays !== retention && <p className="muted">The new retention applies to logs indexed from now on.</p>}
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {index ? 'Save' : 'Create Index'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
