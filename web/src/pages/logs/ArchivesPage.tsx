import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Archive, CheckCircle2, Plus, X, XCircle } from 'lucide-react';
import { archiveRuns, createArchive, deleteArchive, runArchive, testArchive, updateArchive, type ArchiveTarget, type LogArchive } from '../../api/archives/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { useProfile } from '../settings/queries';
import { formatSize, useArchives } from './archiveBits';

/** Log Configuration → Archiving & Forwarding: hourly Parquet archives in S3-compatible buckets. */
export function ArchivesPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  const archives = useArchives();
  const [creating, setCreating] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: ['archives'] });
  const update = useMutation({ mutationFn: (v: { id: string; enabled?: boolean }) => updateArchive(v.id, v), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteArchive, onSuccess: refresh });
  const run = useMutation({ mutationFn: runArchive, onSuccess: refresh });

  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Archiving &amp; Forwarding</h1>
        {isAdmin && (
          <button className="btn btn-primary" onClick={() => setCreating(true)}>
            <Plus size={14} /> New Archive
          </button>
        )}
      </div>
      <p className="muted">
        Every hour, logs are written to your bucket as Parquet (<code>org=…/dt=YYYY-MM-DD/hour=HH/</code>). They stay there after leaving the live index (15 days): search them with Archive Search or
        rehydrate them for the Log Explorer. Any S3-compatible store works — AWS S3, RustFS, Ceph, Cloudflare R2.
      </p>
      {(update.error || remove.error || run.error) && <p className="form-error">{((update.error ?? remove.error ?? run.error) as Error).message}</p>}
      {run.data && (
        <p className="ar-note">
          Archived {run.data.hours} hour{run.data.hours === 1 ? '' : 's'} ({run.data.rows.toLocaleString()} logs).
        </p>
      )}
      {archives.isPending ? (
        <Loading />
      ) : archives.isError ? (
        <ErrorState error={archives.error} onRetry={() => archives.refetch()} />
      ) : archives.data.length === 0 ? (
        <section className="settings-card settings-empty">
          <Archive size={28} />
          <p className="muted">No archives yet. Logs older than 15 days are deleted unless you archive them.</p>
        </section>
      ) : (
        <table className="settings-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>DESTINATION</th>
              <th>FILTER</th>
              <th>STATUS</th>
              <th className="num">HOURS</th>
              <th className="num">LOGS</th>
              <th className="num">SIZE</th>
              <th>LATEST HOUR</th>
              {isAdmin && <th />}
            </tr>
          </thead>
          <tbody>
            {archives.data.map((a) => (
              <ArchiveRow
                key={a.id}
                a={a}
                isAdmin={isAdmin}
                open={open === a.id}
                onToggle={() => setOpen((o) => (o === a.id ? null : a.id))}
                onEnable={(enabled) => update.mutate({ id: a.id, enabled })}
                onRun={() => run.mutate(a.id)}
                running={run.isPending && run.variables === a.id}
                onDelete={() => window.confirm(`Stop archiving to "${a.name}"? Files already in the bucket stay there.`) && remove.mutate(a.id)}
              />
            ))}
          </tbody>
        </table>
      )}
      {creating && <CreateArchiveModal onClose={() => setCreating(false)} onCreated={() => (setCreating(false), void refresh())} />}
    </div>
  );
}

function ArchiveRow({
  a,
  isAdmin,
  open,
  onToggle,
  onEnable,
  onRun,
  running,
  onDelete,
}: {
  a: LogArchive;
  isAdmin: boolean;
  open: boolean;
  onToggle: () => void;
  onEnable: (v: boolean) => void;
  onRun: () => void;
  running: boolean;
  onDelete: () => void;
}) {
  return (
    <>
      <tr>
        <td>
          <button className="link-btn" onClick={onToggle}>
            {a.name}
          </button>
        </td>
        <td className="mono ellipsis" title={a.endpoint}>
          {a.bucket}/{a.prefix}
        </td>
        <td className="mono">{a.query || <span className="muted">all logs</span>}</td>
        <td>
          {!a.enabled ? (
            <span className="pill-chip small">Paused</span>
          ) : a.lastError && a.hoursFailed > 0 ? (
            <span className="ar-status bad" title={a.lastError}>
              <XCircle size={14} /> Failing
            </span>
          ) : (
            <span className="ar-status ok">
              <CheckCircle2 size={14} /> Active
            </span>
          )}
        </td>
        <td className="num">{a.hoursDone}</td>
        <td className="num">{a.rows.toLocaleString()}</td>
        <td className="num">{formatSize(a.bytes)}</td>
        <td className="muted">{a.lastHour ? new Date(a.lastHour).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—'}</td>
        {isAdmin && (
          <td className="col-actions">
            <Menu
              trigger={<span aria-label="Archive actions">⋯</span>}
              triggerClassName="icon-plain"
              items={[
                { label: running ? 'Archiving…' : 'Archive pending hours now', onClick: onRun, disabled: running || !a.enabled },
                { label: a.enabled ? 'Pause' : 'Resume', onClick: () => onEnable(!a.enabled) },
                { label: 'Delete', onClick: onDelete },
              ]}
            />
          </td>
        )}
      </tr>
      {open && (
        <tr className="ar-runs-row">
          <td colSpan={isAdmin ? 9 : 8}>
            <Runs archive={a} />
          </td>
        </tr>
      )}
    </>
  );
}

function Runs({ archive }: { archive: LogArchive }) {
  const q = useQuery({ queryKey: ['archive-runs', archive.id], queryFn: () => archiveRuns(archive.id) });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} />;
  return (
    <div className="ar-runs">
      <p className="muted">
        {archive.endpoint} · region {archive.region} · {archive.pathStyle ? 'path-style' : 'virtual-host'} URLs · key {archive.accessKey}
      </p>
      <table className="settings-table">
        <thead>
          <tr>
            <th>HOUR</th>
            <th>STATUS</th>
            <th className="num">LOGS</th>
            <th className="num">SIZE</th>
            <th>OBJECT</th>
            <th>WRITTEN</th>
          </tr>
        </thead>
        <tbody>
          {q.data.slice(0, 48).map((r) => (
            <tr key={r.hour}>
              <td>{new Date(r.hour).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</td>
              <td>
                {r.status === 'done' ? (
                  'Written'
                ) : (
                  <span className="ar-status bad" title={r.error ?? ''}>
                    Failed ({r.attempts}×)
                  </span>
                )}
              </td>
              <td className="num">{r.rows.toLocaleString()}</td>
              <td className="num">{r.bytes ? formatSize(r.bytes) : '—'}</td>
              <td className="mono ellipsis" title={r.objectKey ?? r.error ?? ''}>
                {r.objectKey ?? (r.error ? r.error : <span className="muted">no logs in this hour</span>)}
              </td>
              <td className="muted">{timeAgo(r.finishedAt)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const EMPTY: ArchiveTarget = { endpoint: '', region: 'us-east-1', bucket: '', prefix: 'logs', pathStyle: true, accessKey: '', secretKey: '' };

function CreateArchiveModal({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const [name, setName] = useState('');
  const [t, setT] = useState<ArchiveTarget>(EMPTY);
  const [query, setQuery] = useState('');
  const [backfill, setBackfill] = useState(1);
  const set = (p: Partial<ArchiveTarget>) => setT((x) => ({ ...x, ...p }));
  const test = useMutation({ mutationFn: () => testArchive(t) });
  const create = useMutation({ mutationFn: () => createArchive({ name, ...t, query, backfillDays: backfill }), onSuccess: onCreated });
  return (
    <Modal onClose={onClose} label="New archive" width={620} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>New Archive</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Production logs" maxLength={100} required autoFocus />
        </label>
        <label className="form-label">
          S3 endpoint
          <input value={t.endpoint} onChange={(e) => set({ endpoint: e.target.value })} placeholder="https://s3.eu-west-1.amazonaws.com" required />
        </label>
        <div className="ar-grid">
          <label className="form-label">
            Bucket
            <input value={t.bucket} onChange={(e) => set({ bucket: e.target.value.toLowerCase() })} placeholder="acme-log-archive" required />
          </label>
          <label className="form-label">
            Prefix
            <input value={t.prefix} onChange={(e) => set({ prefix: e.target.value })} placeholder="logs" />
          </label>
          <label className="form-label">
            Region
            <input value={t.region} onChange={(e) => set({ region: e.target.value })} />
          </label>
        </div>
        <div className="ar-grid">
          <label className="form-label">
            Access key
            <input value={t.accessKey} onChange={(e) => set({ accessKey: e.target.value })} autoComplete="off" required />
          </label>
          <label className="form-label">
            Secret key
            <input type="password" value={t.secretKey} onChange={(e) => set({ secretKey: e.target.value })} autoComplete="new-password" required />
          </label>
        </div>
        <label className="ar-check">
          <input type="checkbox" checked={t.pathStyle} onChange={(e) => set({ pathStyle: e.target.checked })} /> Path-style URLs (endpoint/bucket/key) — needed by most S3-compatible stores
        </label>
        <label className="form-label">
          Logs to archive
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="all logs (or e.g. env:prod -status:debug)" />
        </label>
        <label className="form-label">
          Also archive the last
          <select value={backfill} onChange={(e) => setBackfill(Number(e.target.value))}>
            {[0, 1, 2].map((d) => (
              <option key={d} value={d}>
                {d === 0 ? 'nothing (from now on)' : `${d} day${d === 1 ? '' : 's'} already received`}
              </option>
            ))}
          </select>
        </label>
        {test.data && <p className={test.data.ok ? 'ar-note' : 'form-error'}>{test.data.ok ? 'The bucket is reachable: ObserveCraft can write, read and delete objects there.' : test.data.error}</p>}
        {create.error && <p className="form-error">{(create.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={() => test.mutate()} disabled={test.isPending || !t.endpoint || !t.bucket}>
            {test.isPending ? 'Testing…' : 'Test Connection'}
          </button>
          <button className="btn btn-primary" disabled={create.isPending}>
            {create.isPending ? 'Checking bucket…' : 'Create Archive'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
