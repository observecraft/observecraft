import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Archive as ArchiveIcon, Download, History, Plus, Search, X } from 'lucide-react';
import { expireRehydration, listRehydrations, rehydrate, type Rehydration } from '../../api/archives/client';
import { searchLogPage } from '../../api/client';
import type { LogEvent } from '../../api/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { formatRange, formatSize, RangeFields, useArchives } from './archiveBits';
import { Avatar, Columns, ConfigTitle, downloadCsv, FilterQuery, FlowStep, SectionHead } from './config/configUi';
import { LogDetail } from './LogDetail';
import { LogTable } from './LogTable';

const HOUR = 3_600_000;

const STATUS: Record<string, [string, string]> = { running: ['LOADING', 'info'], done: ['READY', 'ok'], failed: ['FAILED', 'error'], expired: ['EXPIRED', 'muted'] };

/** Log Configuration → Rehydrations: archived logs loaded back for the Log Explorer (`index:rehydrated`). */
export function RehydrationsPage() {
  const qc = useQueryClient();
  const list = useQuery({ queryKey: ['rehydrations'], queryFn: listRehydrations, refetchInterval: (q) => (q.state.data?.some((r) => r.status === 'running') ? 2000 : false) });
  const expire = useMutation({ mutationFn: expireRehydration, onSuccess: () => qc.invalidateQueries({ queryKey: ['rehydrations'] }) });
  const [creating, setCreating] = useState(false);
  const [viewing, setViewing] = useState<Rehydration | null>(null);
  const [view, setView] = useState('current');
  const [search, setSearch] = useState('');
  const [open, setOpen] = useState(true);
  const all = list.data ?? [];
  const needle = search.trim().toLowerCase();
  const shown = all.filter(
    (r) => (view === 'all' || (view === 'current' ? r.status !== 'expired' : r.status === 'expired')) && (!needle || r.name.toLowerCase().includes(needle) || r.query.toLowerCase().includes(needle)),
  );
  const template = '96px minmax(180px, 1.2fr) minmax(260px, 2fr) 90px 100px 130px 34px 120px';
  const csv = () =>
    downloadCsv('rehydrations.csv', [
      ['name', 'archive', 'query', 'from', 'to', 'status', 'scan bytes', 'events', 'created', 'by'],
      ...shown.map((r) => [
        r.name,
        r.archiveName,
        r.query,
        new Date(r.from).toISOString(),
        new Date(r.to).toISOString(),
        r.status,
        r.bytesScanned,
        r.rows,
        new Date(r.createdAt).toISOString(),
        r.createdBy ?? '',
      ]),
    ]);
  return (
    <div className="lc-page">
      <section className="lc-banner">
        <Link to="/logs/archive-search">Archive Search</Link> queries archived logs where they are, without loading them: try it first, and rehydrate when you need dashboards, facets or the Log
        Explorer on the results.
      </section>
      <ConfigTitle
        title="Rehydrations"
        actions={
          <button className="btn btn-primary" onClick={() => setCreating(true)}>
            <Plus size={14} /> New Historical View
          </button>
        }
      />
      <div className="lc-view">
        <label className="lc-view-select">
          <span>View</span>
          <select value={view} onChange={(e) => setView(e.target.value)} aria-label="View">
            <option value="current">Current historical views</option>
            <option value="expired">Expired historical views</option>
            <option value="all">All historical views</option>
          </select>
        </label>
        <div className="lc-search">
          <Search size={14} className="muted" />
          <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Filter Historical Views" aria-label="Filter historical views" />
        </div>
      </div>
      <div className="lc-count">
        <b>{all.length}</b> historical views defined
      </div>
      <div className="lc-flow">
        <div className="lc-flow-box">
          <FlowStep icon={<ArchiveIcon size={15} />} label="Archives" to="/logs/archives" last />
        </div>
        <div className="lc-flow-link" />
      </div>
      {expire.error && <p className="form-error">{(expire.error as Error).message}</p>}
      {list.isPending ? (
        <Loading />
      ) : list.isError ? (
        <ErrorState error={list.error} onRetry={() => list.refetch()} />
      ) : (
        <section className="lc-section">
          <SectionHead
            open={open}
            onToggle={() => setOpen((o) => !o)}
            label="HISTORICAL VIEW"
            right={
              <button className="link-btn" onClick={csv} disabled={shown.length === 0}>
                <Download size={13} /> Download CSV
              </button>
            }
          >
            <span>
              <b>{shown.length}</b> view{shown.length === 1 ? '' : 's'}
            </span>
          </SectionHead>
          {open && (
            <>
              <Columns template={template} cols={['', 'Name', 'Query', 'Scan size', '# of events', 'Created', 'By', '']} />
              {shown.map((r) => {
                const [label, tone] = STATUS[r.status] ?? [r.status.toUpperCase(), 'muted'];
                return (
                  <div key={r.id} className={`lc-row${r.status === 'expired' ? ' dim' : ''}`} style={{ gridTemplateColumns: template }}>
                    <span className="lc-keep" title={r.error ?? (r.status === 'done' ? `Searchable until ${new Date(r.expiresAt).toLocaleString()}` : '')}>
                      <span className={`lc-pill ${tone}`}>{label}</span>
                    </span>
                    <span className="lc-cell-name">
                      {r.status === 'done' ? (
                        <button className="link-btn" onClick={() => setViewing(r)} title={r.name}>
                          {r.name}
                        </button>
                      ) : (
                        <span title={r.name}>{r.name}</span>
                      )}
                    </span>
                    <span className="lc-stack" title={`${r.archiveName}: ${r.query || '*'}`}>
                      <FilterQuery query={r.query} />
                      <span className="muted">
                        {r.archiveName} · {formatRange(r.from, r.to)}
                      </span>
                    </span>
                    <span>{formatSize(r.bytesScanned)}</span>
                    <span>{r.rows.toLocaleString()}</span>
                    <span className="muted">{new Date(r.createdAt).toLocaleString(undefined, { month: 'short', day: '2-digit', hour: '2-digit', minute: '2-digit' })}</span>
                    <Avatar name={r.createdBy} />
                    <span className="lc-actions lc-keep">
                      {r.status === 'done' && (
                        <button className="btn btn-sm" onClick={() => window.confirm(`Remove the rehydrated logs of "${r.name}" now?`) && expire.mutate(r.id)}>
                          Expire now
                        </button>
                      )}
                    </span>
                  </div>
                );
              })}
              {all.length === 0 && (
                <p className="lc-empty">
                  <History size={22} />
                  <br />
                  Nothing rehydrated yet.
                </p>
              )}
              <button className="lc-add-row" onClick={() => setCreating(true)}>
                <Plus size={14} /> Add a new historical view
              </button>
            </>
          )}
        </section>
      )}
      <p className="muted lc-note">
        A historical view loads archived logs back into the live store for a few days, so dashboards, facets and the Log Explorer work on them. Search them with <code>index:rehydrated</code>.
      </p>
      {viewing && <RehydratedLogs r={viewing} onClose={() => setViewing(null)} />}
      {creating && <RehydrateModal onClose={() => setCreating(false)} onDone={() => (setCreating(false), void qc.invalidateQueries({ queryKey: ['rehydrations'] }))} />}
    </div>
  );
}

/** The rehydrated logs over their own (possibly old) time range. */
function RehydratedLogs({ r, onClose }: { r: Rehydration; onClose: () => void }) {
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<LogEvent | null>(null);
  const q = useQuery({ queryKey: ['rehydrated-logs', r.id, query], queryFn: () => searchLogPage({ range: { from: r.from, to: r.to }, query: `index:rehydrated ${query}`.trim() }) });
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
          <h3>New Historical View</h3>
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
