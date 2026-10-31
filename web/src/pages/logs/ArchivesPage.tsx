import { useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Archive, CheckCircle2, Globe, Plus, Search, Workflow, X, XCircle } from 'lucide-react';
import { archiveRuns, createArchive, deleteArchive, runArchive, testArchive, updateArchive, type ArchiveTarget, type LogArchive } from '../../api/archives/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';
import { compact, timeAgo } from '../../lib/format';
import { useProfile } from '../settings/queries';
import { formatSize, useArchives } from './archiveBits';
import { Columns, ConfigTitle, FilterQuery, FlowStep, Num, SectionHead, TagChips } from './config/configUi';
import { DestinationsList } from './DestinationsList';

/** Log Configuration → Archiving & Forwarding: hourly Parquet archives in S3-compatible buckets, and custom destinations. */
export function ArchivesPage() {
  const [params, setParams] = useSearchParams();
  const tab = params.get('tab') === 'destinations' ? 'destinations' : 'archives';
  const choose = (t: string) => setParams(t === 'archives' ? {} : { tab: t }, { replace: true });
  return (
    <div className="lc-page">
      <ConfigTitle title="Archiving & Forwarding" />
      <div className="lc-choice">
        <button className={tab === 'archives' ? 'active' : ''} onClick={() => choose('archives')}>
          <b>Archives</b>
          <span>Write logs to your own S3-compatible or cloud storage</span>
        </button>
        <button className={tab === 'destinations' ? 'active' : ''} onClick={() => choose('destinations')}>
          <b>Custom Destinations</b>
          <span>Forward logs to other tools, such as your SIEM</span>
        </button>
      </div>
      {tab === 'archives' ? <ArchivesList /> : <DestinationsList />}
    </div>
  );
}

function ArchivesList() {
  const qc = useQueryClient();
  const me = useProfile();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  const archives = useArchives();
  const [creating, setCreating] = useState(false);
  const [keys, setKeys] = useState<LogArchive | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [section, setSection] = useState(true);
  const [view, setView] = useState('all');
  const [search, setSearch] = useState('');
  const refresh = () => qc.invalidateQueries({ queryKey: ['archives'] });
  const update = useMutation({ mutationFn: (v: { id: string; enabled?: boolean }) => updateArchive(v.id, v), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteArchive, onSuccess: refresh });
  const run = useMutation({ mutationFn: runArchive, onSuccess: refresh });

  if (archives.isPending) return <Loading />;
  if (archives.isError) return <ErrorState error={archives.error} onRetry={() => archives.refetch()} />;
  const all = archives.data;
  const needle = search.trim().toLowerCase();
  const shown = all.filter(
    (a) =>
      (!needle || a.name.toLowerCase().includes(needle) || a.query.toLowerCase().includes(needle) || a.bucket.includes(needle)) &&
      (view === 'all' || (view === 'active' && a.enabled) || (view === 'paused' && !a.enabled) || (view === 'keys' && !a.hasCredentials)),
  );
  const active = all.filter((a) => a.enabled).length;
  const template = '24px minmax(150px, 1fr) minmax(180px, 1.4fr) minmax(170px, 1.2fr) 120px 90px 80px 130px 30px';
  return (
    <>
      <div className="lc-title-row">
        <div className="lc-count">
          <b>{all.length}</b> archives defined
        </div>
        {isAdmin && (
          <button className="btn btn-primary" onClick={() => setCreating(true)}>
            <Plus size={14} /> New Archive
          </button>
        )}
      </div>
      <div className="lc-view">
        <label className="lc-view-select">
          <span>View</span>
          <select value={view} onChange={(e) => setView(e.target.value)} aria-label="View">
            <option value="all">All archives</option>
            <option value="active">Active</option>
            <option value="paused">Paused</option>
            <option value="keys">Waiting for keys</option>
          </select>
        </label>
        <div className="lc-search">
          <Search size={14} className="muted" />
          <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Filter Archives" aria-label="Filter archives" />
        </div>
      </div>
      <div className="lc-flow">
        <div className="lc-flow-box">
          <FlowStep icon={<Workflow size={15} />} label="Pipelines" to="/logs/pipelines" right="Every log received, indexed or not, processed by the pipelines" last />
        </div>
        <div className="lc-flow-link" />
      </div>
      {(update.error || remove.error || run.error) && <p className="form-error">{((update.error ?? remove.error ?? run.error) as Error).message}</p>}
      {run.data && (
        <p className="ar-note">
          Archived {run.data.hours} hour{run.data.hours === 1 ? '' : 's'} ({run.data.rows.toLocaleString()} logs).
        </p>
      )}
      <section className="lc-section">
        <SectionHead open={section} onToggle={() => setSection((o) => !o)} label="NAME">
          <span>
            <b>{active}</b> ACTIVE
          </span>
          <span className="sep">|</span>
          <span>
            <b>{all.length - active}</b> PAUSED
          </span>
        </SectionHead>
        {section && (
          <>
            <Columns template={template} cols={['', 'Name', 'Filters', 'Destination', 'Status', 'Logs', 'Size', 'Latest hour', '']} />
            {shown.map((a) => {
              const isOpen = open === a.id;
              return (
                <div key={a.id}>
                  <div className={`lc-row${a.enabled ? '' : ' dim'}`} style={{ gridTemplateColumns: template }}>
                    <Num n={all.indexOf(a) + 1} muted={!a.enabled} />
                    <span className="lc-cell-name">
                      <button onClick={() => setOpen(isOpen ? null : a.id)} aria-expanded={isOpen}>
                        {a.name}
                      </button>
                    </span>
                    <FilterQuery query={a.query} />
                    <span className="lc-cell-name" title={`${a.endpoint || a.destinationType}/${a.bucket}/${a.prefix}`}>
                      <DestinationLogo kind={a.destinationType} />
                      <span className="mono">
                        {a.bucket}/{a.prefix}
                      </span>
                    </span>
                    <span className="lc-keep">
                      {!a.hasCredentials ? (
                        <button className="lc-pill warn" onClick={() => isAdmin && setKeys(a)} title="Created through the API: set the bucket's keys to start archiving" disabled={!isAdmin}>
                          Needs keys
                        </button>
                      ) : !a.enabled ? (
                        <span className="lc-pill muted">Paused</span>
                      ) : a.lastError && a.hoursFailed > 0 ? (
                        <span className="ar-status bad" title={a.lastError}>
                          <XCircle size={14} /> Failing
                        </span>
                      ) : (
                        <span className="ar-status ok">
                          <CheckCircle2 size={14} /> Active
                        </span>
                      )}
                    </span>
                    <span>{compact(a.rows)}</span>
                    <span>{formatSize(a.bytes)}</span>
                    <span className="muted">{a.lastHour ? new Date(a.lastHour).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—'}</span>
                    <span className="lc-keep">
                      {isAdmin && (
                        <Menu
                          trigger={<span aria-label="Archive actions">⋯</span>}
                          triggerClassName="icon-plain"
                          items={[
                            {
                              label: run.isPending && run.variables === a.id ? 'Archiving…' : 'Archive pending hours now',
                              onClick: () => run.mutate(a.id),
                              disabled: run.isPending || !a.enabled || !a.hasCredentials,
                            },
                            { label: a.hasCredentials ? 'Change bucket keys' : 'Set bucket keys', onClick: () => setKeys(a) },
                            { label: a.enabled ? 'Pause' : 'Resume', onClick: () => update.mutate({ id: a.id, enabled: !a.enabled }) },
                            { label: 'Delete', onClick: () => window.confirm(`Stop archiving to "${a.name}"? Files already in the bucket stay there.`) && remove.mutate(a.id) },
                          ]}
                        />
                      )}
                    </span>
                  </div>
                  {isOpen && (
                    <div className="lc-detail">
                      <dl className="lc-kv">
                        <dt>Tags</dt>
                        <dd>{a.includeTags ? 'Kept with each archived log' : 'Requested to be dropped (tags are archived)'}</dd>
                        {a.rehydrationTags.length > 0 && (
                          <>
                            <dt>Rehydration tags</dt>
                            <dd>
                              <TagChips tags={a.rehydrationTags} max={10} />
                            </dd>
                          </>
                        )}
                        <dt>Rehydration scan limit</dt>
                        <dd>{a.rehydrationMaxScanSizeGb ? `${a.rehydrationMaxScanSizeGb} GB` : 'None'}</dd>
                      </dl>
                      {a.hasCredentials ? <Runs archive={a} /> : <p className="muted">Nothing is archived until the bucket's keys are set.</p>}
                    </div>
                  )}
                </div>
              );
            })}
            {all.length === 0 && (
              <p className="lc-empty">
                <Archive size={22} />
                <br />
                No archives yet. Logs leave their index when its retention ends, unless you archive them.
              </p>
            )}
            {all.length > 0 && shown.length === 0 && <p className="lc-empty">No archive matches.</p>}
          </>
        )}
      </section>
      <p className="muted lc-note">
        Every hour, logs are written to the bucket as Parquet (<code>org=…/dt=YYYY-MM-DD/hour=HH/</code>). Search them with Archive Search or rehydrate them for the Log Explorer. Any S3-compatible
        store works: AWS S3, Cloud Storage (HMAC keys), RustFS, Ceph, Cloudflare R2.
      </p>
      {creating && <CreateArchiveModal onClose={() => setCreating(false)} onCreated={() => (setCreating(false), void refresh())} />}
      {keys && <KeysModal archive={keys} onClose={() => setKeys(null)} onSaved={() => (setKeys(null), void refresh())} />}
    </>
  );
}

function DestinationLogo({ kind }: { kind: string }) {
  const brand: Record<string, string> = { s3: 'aws', gcs: 'googlecloud', azure: 'azure', splunk_hec: 'splunk', elasticsearch: 'elasticsearch', microsoft_sentinel: 'azure' };
  const name = brand[kind];
  return name && hasBrandLogo(name) ? <BrandLogo name={name} size={16} title={kind} /> : <Globe size={14} className="muted" />;
}

/** Bucket and keys of an archive (archives created through Datadog's API carry none). */
function KeysModal({ archive, onClose, onSaved }: { archive: LogArchive; onClose: () => void; onSaved: () => void }) {
  const [t, setT] = useState<ArchiveTarget>({
    endpoint: archive.endpoint || 'https://s3.amazonaws.com',
    region: archive.region || 'us-east-1',
    bucket: archive.bucket,
    prefix: archive.prefix,
    pathStyle: archive.pathStyle,
    accessKey: archive.accessKey,
    secretKey: '',
  });
  const set = (p: Partial<ArchiveTarget>) => setT((x) => ({ ...x, ...p }));
  const save = useMutation({ mutationFn: () => updateArchive(archive.id, { target: t }), onSuccess: onSaved });
  return (
    <Modal onClose={onClose} label="Bucket keys" width={560} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>Bucket keys of {archive.name}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        {archive.destinationType === 'azure' && <p className="form-error">Azure Blob Storage has no S3-compatible API: point this archive at an S3-compatible bucket instead.</p>}
        <label className="form-label">
          S3 endpoint
          <input value={t.endpoint} onChange={(e) => set({ endpoint: e.target.value })} required />
        </label>
        <div className="ar-grid">
          <label className="form-label">
            Bucket
            <input value={t.bucket} onChange={(e) => set({ bucket: e.target.value.toLowerCase() })} required />
          </label>
          <label className="form-label">
            Prefix
            <input value={t.prefix} onChange={(e) => set({ prefix: e.target.value })} />
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
          <input type="checkbox" checked={t.pathStyle} onChange={(e) => set({ pathStyle: e.target.checked })} /> Path-style URLs (endpoint/bucket/key)
        </label>
        <p className="muted">The bucket is checked by writing, reading and deleting a test object before the keys are saved.</p>
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {save.isPending ? 'Checking bucket…' : 'Save Keys'}
          </button>
        </div>
      </form>
    </Modal>
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
