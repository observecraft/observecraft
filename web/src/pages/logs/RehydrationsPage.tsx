import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { History, Plus, X } from 'lucide-react';
import { expireRehydration, listRehydrations, rehydrate, type Rehydration } from '../../api/archives/client';
import { searchLogs } from '../../api/client';
import type { LogEvent } from '../../api/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { formatRange, RangeFields, useArchives } from './archiveBits';
import { LogDetail } from './LogDetail';
import { LogTable } from './LogTable';

const HOUR = 3_600_000;

/** Log Configuration → Rehydrations: archived logs loaded back for the Log Explorer (`index:rehydrated`). */
export function RehydrationsPage() {
  const qc = useQueryClient();
  const list = useQuery({ queryKey: ['rehydrations'], queryFn: listRehydrations, refetchInterval: (q) => (q.state.data?.some((r) => r.status === 'running') ? 2000 : false) });
  const expire = useMutation({ mutationFn: expireRehydration, onSuccess: () => qc.invalidateQueries({ queryKey: ['rehydrations'] }) });
  const [creating, setCreating] = useState(false);
  const [viewing, setViewing] = useState<Rehydration | null>(null);
  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Rehydrations</h1>
        <button className="btn btn-primary" onClick={() => setCreating(true)}>
          <Plus size={14} /> Rehydrate From Archive
        </button>
      </div>
      <p className="muted">
        Loads archived logs back into the live store for a few days, so dashboards, facets and the Log Explorer work on them. Search them with <code>index:rehydrated</code>.
      </p>
      {expire.error && <p className="form-error">{(expire.error as Error).message}</p>}
      {list.isPending ? (
        <Loading />
      ) : list.isError ? (
        <ErrorState error={list.error} onRetry={() => list.refetch()} />
      ) : list.data.length === 0 ? (
        <section className="settings-card settings-empty">
          <History size={28} />
          <p className="muted">Nothing rehydrated yet.</p>
        </section>
      ) : (
        <table className="settings-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>ARCHIVE</th>
              <th>QUERY</th>
              <th>TIME RANGE</th>
              <th>STATUS</th>
              <th className="num">LOGS</th>
              <th>EXPIRES</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {list.data.map((r) => (
              <tr key={r.id} className={r.status === 'expired' ? 'row-muted' : ''}>
                <td>
                  {r.status === 'done' ? (
                    <button className="link-btn" onClick={() => setViewing(r)}>
                      {r.name}
                    </button>
                  ) : (
                    r.name
                  )}
                </td>
                <td>{r.archiveName}</td>
                <td className="mono">{r.query || <span className="muted">all logs</span>}</td>
                <td className="muted">{formatRange(r.from, r.to)}</td>
                <td title={r.error ?? ''}>
                  {r.status === 'running' ? 'Loading…' : r.status === 'failed' ? <span className="ar-status bad">Failed</span> : r.status === 'expired' ? 'Expired' : 'Ready'}
                </td>
                <td className="num">{r.rows.toLocaleString()}</td>
                <td className="muted">{r.status === 'done' ? new Date(r.expiresAt).toLocaleDateString() : '—'}</td>
                <td className="num">
                  {r.status === 'done' && (
                    <button className="btn btn-sm" onClick={() => window.confirm(`Remove the rehydrated logs of "${r.name}" now?`) && expire.mutate(r.id)}>
                      Expire now
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {viewing && <RehydratedLogs r={viewing} onClose={() => setViewing(null)} />}
      {creating && <RehydrateModal onClose={() => setCreating(false)} onDone={() => (setCreating(false), void qc.invalidateQueries({ queryKey: ['rehydrations'] }))} />}
    </div>
  );
}

/** The rehydrated logs over their own (possibly old) time range. */
function RehydratedLogs({ r, onClose }: { r: Rehydration; onClose: () => void }) {
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<LogEvent | null>(null);
  const q = useQuery({ queryKey: ['rehydrated-logs', r.id, query], queryFn: () => searchLogs({ range: { from: r.from, to: r.to }, query: `index:rehydrated ${query}`.trim() }) });
  return (
    <section className="ar-result">
      <div className="ar-result-head">
        <div>
          <b>{r.name}</b> <span className="muted">{formatRange(r.from, r.to)}</span>
        </div>
        <div className="ar-progress">
          <input className="ar-inline-query" placeholder="Filter (search syntax)" value={query} onChange={(e) => setQuery(e.target.value)} />
          <button className="icon-plain" title="Close" onClick={onClose}>
            <X size={16} />
          </button>
        </div>
      </div>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} />
      ) : (
        <>
          <p className="muted">{q.data.total.toLocaleString()} logs</p>
          <div className="ar-table">
            <LogTable logs={q.data.logs} selectedId={selected?.id ?? null} onSelect={setSelected} />
          </div>
        </>
      )}
      {selected && <LogDetail log={selected} onClose={() => setSelected(null)} onFilter={(k, v) => setQuery((x) => `${x} ${k}:${v}`.trim())} />}
    </section>
  );
}

function RehydrateModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const archives = useArchives();
  const [archiveId, setArchiveId] = useState('');
  const [name, setName] = useState('');
  const [query, setQuery] = useState('');
  const [range, setRange] = useState(() => ({ from: Date.now() - 24 * HOUR, to: Date.now() }));
  const [days, setDays] = useState(7);
  const go = useMutation({ mutationFn: () => rehydrate({ archiveId: archiveId || archives.data![0].id, name, query, range, retentionDays: days }), onSuccess: onDone });
  return (
    <Modal onClose={onClose} label="Rehydrate" width={620} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          go.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>Rehydrate From Archive</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        {archives.data?.length === 0 && <p className="form-error">Set up an archive first (Archiving &amp; Forwarding).</p>}
        <label className="form-label">
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Checkout errors during the outage" maxLength={100} required autoFocus />
        </label>
        <label className="form-label">
          Archive
          <select value={archiveId || archives.data?.[0]?.id || ''} onChange={(e) => setArchiveId(e.target.value)}>
            {archives.data?.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
        </label>
        <label className="form-label">
          Query
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="all logs (or e.g. service:order-service status:error)" />
        </label>
        <RangeFields range={range} onChange={setRange} />
        <label className="form-label">
          Keep for
          <select value={days} onChange={(e) => setDays(Number(e.target.value))}>
            {[1, 3, 7, 14, 30].map((d) => (
              <option key={d} value={d}>
                {d} day{d === 1 ? '' : 's'}
              </option>
            ))}
          </select>
        </label>
        {go.error && <p className="form-error">{(go.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={go.isPending || !archives.data?.length}>
            Rehydrate
          </button>
        </div>
      </form>
    </Modal>
  );
}
