import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { AlertTriangle, ArrowRight, Bell, ChevronDown, ChevronLeft, ChevronRight, Code2, Copy, FileText, LayoutDashboard, Search, Trash2, UploadCloud } from 'lucide-react';
import { createImport, deleteImport, getImport, listImports, migrateImport, type ImportAsset, type ImportStatus, type LogImportDetail } from '../../../api/logMigration/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { formatLongDate, timeAgo } from '../../../lib/format';
import { describeAggregation, explorerLink } from './TranslatePage';
import './migration.css';

// The searches users run in Splunk to export what we import. Our own; each exports one CSV.
const EXPORT_ALERTS = `| rest /servicesNS/-/-/saved/searches splunk_server=local count=0
| search disabled=0
| table title search description is_scheduled cron_schedule dispatch.earliest_time dispatch.latest_time alert_type alert_comparator alert_threshold actions action.email.to`;
const EXPORT_DASHBOARDS = `| rest /servicesNS/-/-/data/ui/views splunk_server=local count=0
| search isDashboard=1 isVisible=1
| table title label eai:acl.app eai:data`;
const EXPORT_MACROS = `| rest /servicesNS/-/-/admin/macros splunk_server=local count=0
| table title definition args`;

const STATUS_LABEL: Record<ImportStatus, string> = { ready: 'READY', partial: 'NEEDS REVIEW', unsupported: 'NOT SUPPORTED', migrated: 'MIGRATED', failed: 'FAILED' };

function StatusPill({ s }: { s: ImportStatus }) {
  return <span className={`mig-status mig-${s}`}>{STATUS_LABEL[s]}</span>;
}

// ================================================================ landing

/** Logs → Configuration → Migration → Import: start a migration and follow the ones in progress. */
export function ImportsPage() {
  const navigate = useNavigate();
  const [q, setQ] = useState('');
  const list = useQuery({ queryKey: ['log-imports'], queryFn: listImports });
  const rows = (list.data ?? []).filter((i) => !q.trim() || i.name.toLowerCase().includes(q.trim().toLowerCase()));
  return (
    <div className="settings-pad mig-page">
      <section className="mig-hero">
        <div className="mig-eyebrow">IMPORT · REVIEW · MIGRATE</div>
        <h1>Bring your Splunk searches to ObserveCraft</h1>
        <p className="muted">Translate one search interactively, or import your alerts and dashboards in bulk and migrate them after a review.</p>
        <div className="mig-hero-cards">
          <div className="mig-hero-card">
            <div className="mig-card-kind">
              <Code2 size={14} /> QUICK TRANSLATION
            </div>
            <b>Copy and paste</b>
            <p className="muted">Translate a search and see what each part became, then open it in the Log Explorer.</p>
            <button className="btn" onClick={() => navigate('/logs/translate')}>
              Translate <ArrowRight size={14} />
            </button>
          </div>
          <div className="mig-hero-card">
            <div className="mig-card-kind">
              <UploadCloud size={14} /> BULK MIGRATION
            </div>
            <b>Import your assets</b>
            <p className="muted">Upload exported alerts, dashboards and macros: review every translation, then create the monitors and dashboards.</p>
            <button className="btn btn-primary" onClick={() => navigate('/logs/import/new')}>
              Get Started <ArrowRight size={14} />
            </button>
          </div>
        </div>
      </section>
      <h2 className="mig-h2">Your Imports</h2>
      <label className="mig-search">
        <Search size={14} />
        <input placeholder="Search imports" value={q} onChange={(e) => setQ(e.target.value)} />
      </label>
      {list.isPending ? (
        <Loading />
      ) : list.isError ? (
        <ErrorState error={list.error} onRetry={() => list.refetch()} />
      ) : (
        <table className="settings-table mig-imports">
          <thead>
            <tr>
              <th>NAME</th>
              <th>ASSETS</th>
              <th>PROGRESS</th>
              <th>CREATED AT</th>
              <th>BY</th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 ? (
              <tr>
                <td colSpan={5}>
                  <div className="mig-empty">
                    <Search size={44} className="mig-empty-icon" />
                    <h3>No ongoing migrations</h3>
                    <p className="muted">
                      Import the alerts, dashboards and macros exported from Splunk to translate them to ObserveCraft searches. Include the macros file if your searches use macros.
                    </p>
                    <button className="btn btn-primary" onClick={() => navigate('/logs/import/new')}>
                      Import Resources
                    </button>
                  </div>
                </td>
              </tr>
            ) : (
              rows.map((i) => (
                <tr key={i.id} className="clickable" onClick={() => navigate(`/logs/import/${i.id}`)}>
                  <td>
                    <Link to={`/logs/import/${i.id}`} onClick={(e) => e.stopPropagation()}>
                      <b>{i.name}</b>
                    </Link>
                  </td>
                  <td className="muted">
                    {plural(i.counts.alerts, 'alert')} · {plural(i.counts.dashboards, 'dashboard')} · {plural(i.counts.searches, 'search', 'searches')}
                  </td>
                  <td>
                    <Progress c={i.counts} />
                  </td>
                  <td className="nowrap">{formatLongDate(i.createdAt).split(' at ')[0]}</td>
                  <td>{i.createdBy ?? '—'}</td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      )}
    </div>
  );
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

function Progress({ c }: { c: LogImportDetail['counts'] }) {
  const pct = (n: number) => `${c.total ? (n / c.total) * 100 : 0}%`;
  return (
    <span className="mig-progress" title={`${c.migrated} migrated, ${c.ready} ready, ${c.partial} to review, ${c.unsupported + c.failed} not migrated`}>
      <span className="mig-bar">
        <i className="mig-migrated" style={{ width: pct(c.migrated) }} />
        <i className="mig-ready" style={{ width: pct(c.ready) }} />
        <i className="mig-partial" style={{ width: pct(c.partial) }} />
        <i className="mig-failed" style={{ width: pct(c.unsupported + c.failed) }} />
      </span>
      <small>
        {c.migrated}/{c.total} migrated
      </small>
    </span>
  );
}

// ================================================================ wizard

type Kind = 'alerts' | 'dashboards' | 'macros';
interface Upload {
  name: string;
  kind: Kind;
  text: string;
}

/** The export a CSV comes from, by its header. */
function kindOf(text: string): Kind | null {
  const header = text.replace(/^﻿/, '').split(/\r?\n/, 1)[0].toLowerCase();
  if (header.includes('eai:data')) return 'dashboards';
  if (header.includes('definition')) return 'macros';
  if (header.includes('search')) return 'alerts';
  return null;
}

function CopyBlock({ label, text }: { label: string; text: string }) {
  const [done, setDone] = useState(false);
  return (
    <div className="mig-export">
      <b>{label}</b>
      <pre>{text}</pre>
      <button
        className="btn btn-sm mig-copy"
        aria-label={`Copy the ${label.toLowerCase()} export search`}
        onClick={() => {
          void navigator.clipboard?.writeText(text);
          setDone(true);
          setTimeout(() => setDone(false), 1500);
        }}
      >
        <Copy size={13} /> {done ? 'Copied' : ''}
      </button>
    </div>
  );
}

function DropZone({ accept, onFiles, hint }: { accept: Kind[]; onFiles: (u: Upload[], rejected: string[]) => void; hint: string }) {
  const [over, setOver] = useState(false);
  const read = async (files: File[]) => {
    const ups: Upload[] = [];
    const rejected: string[] = [];
    for (const f of files) {
      const text = await f.text();
      const kind = kindOf(text);
      if (kind && accept.includes(kind)) ups.push({ name: f.name, kind, text });
      else rejected.push(f.name);
    }
    onFiles(ups, rejected);
  };
  return (
    <label
      className={`mig-drop${over ? ' over' : ''}`}
      onDragOver={(e) => {
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setOver(false);
        void read([...e.dataTransfer.files]);
      }}
    >
      <UploadCloud size={30} />
      <span>
        Drop files or <u>browse files</u>
      </span>
      <small className="muted">{hint}</small>
      <small className="muted">
        <kbd>Ctrl</kbd> + <kbd>V</kbd> Paste from Clipboard
      </small>
      <input
        type="file"
        accept=".csv,text/csv"
        multiple
        hidden
        onChange={(e) => {
          void read([...(e.target.files ?? [])]);
          e.target.value = '';
        }}
      />
    </label>
  );
}

/** Splunk → ObserveCraft import: export searches, uploads, name. */
export function NewImportPage() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [uploads, setUploads] = useState<Upload[]>([]);
  const [rejected, setRejected] = useState<string[]>([]);
  const [more, setMore] = useState(false);
  const [name, setName] = useState(() => `Splunk import ${new Date().toISOString().slice(0, 10)}`);
  const add = (u: Upload[], r: string[]) => {
    setUploads((list) => [...list.filter((x) => !u.some((y) => y.name === x.name)), ...u]);
    setRejected(r);
  };
  // Ctrl+V anywhere on the page: the clipboard's CSV text.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest('input, textarea')) return;
      const text = e.clipboardData?.getData('text') ?? '';
      const kind = kindOf(text);
      if (kind) add([{ name: `Pasted ${kind}.csv`, kind, text }], []);
      else if (text.trim()) setRejected(['the pasted text (not one of the exports)']);
    };
    window.addEventListener('paste', onPaste);
    return () => window.removeEventListener('paste', onPaste);
  }, []);
  const join = (k: Kind) => {
    const files = uploads.filter((u) => u.kind === k);
    if (!files.length) return undefined;
    // Several files of one kind: one CSV (the header once).
    return files.map((f, i) => (i === 0 ? f.text : f.text.split(/\r?\n/).slice(1).join('\n'))).join('\n');
  };
  const create = useMutation({
    mutationFn: () => createImport({ name, alerts: join('alerts'), dashboards: join('dashboards'), macros: join('macros') }),
    onSuccess: (d) => {
      void qc.invalidateQueries({ queryKey: ['log-imports'] });
      navigate(`/logs/import/${d.id}`);
    },
  });
  const assets = uploads.filter((u) => u.kind !== 'macros');
  const files = (kinds: Kind[]) =>
    uploads
      .filter((u) => kinds.includes(u.kind))
      .map((u) => (
        <span key={u.name} className="pill-chip">
          <FileText size={12} /> {u.name} <small className="muted">{u.kind}</small>
          <button className="link-muted" aria-label={`Remove ${u.name}`} onClick={() => setUploads((l) => l.filter((x) => x !== u))}>
            ×
          </button>
        </span>
      ));
  return (
    <div className="settings-pad mig-page">
      <Link to="/logs/import" className="mig-back">
        <ChevronLeft size={14} /> Back to Imports
      </Link>
      <div className="mig-source-row">
        <span className="mig-source-tile">Splunk</span>
        <span className="mig-other" title="Splunk is the only source for now">
          Using another source?
        </span>
      </div>
      <p className="muted">
        Upload the CSV exports of your Splunk alerts and dashboards. Include the macros file if your searches use macros: without it, searches that call them can't be translated.
      </p>
      <ol className="mig-steps-list">
        <li>
          <h3>
            Upload your assets from Splunk <span className="mig-req">*</span>
          </h3>
          <div className="mig-instructions">
            <div className="mig-instr-head">
              <FileText size={15} /> <b>Export your assets as CSV files.</b>
              <span className="mig-required">REQUIRED</span>
            </div>
            <p>Run each search in Splunk's search bar and export the results as CSV, then upload the files below.</p>
            <CopyBlock label="Alerts and saved searches" text={EXPORT_ALERTS} />
            {more && <CopyBlock label="Dashboards" text={EXPORT_DASHBOARDS} />}
            <button className="mig-more" onClick={() => setMore((m) => !m)}>
              {more ? 'Show Less' : 'Show More'} <ChevronDown size={14} className={more ? 'flip' : ''} />
            </button>
          </div>
          <DropZone accept={['alerts', 'dashboards']} onFiles={add} hint="Your alerts and dashboards CSV files, as many as you need" />
          <div className="chip-row">{files(['alerts', 'dashboards'])}</div>
        </li>
        <li>
          <h3>Macros file</h3>
          <div className="mig-instructions">
            <p>If your searches use macros, export their definitions too. Without them, those searches can't be expanded and their translation fails.</p>
            <CopyBlock label="Macros" text={EXPORT_MACROS} />
          </div>
          <DropZone accept={['macros']} onFiles={add} hint="The macros CSV file" />
          <div className="chip-row">{files(['macros'])}</div>
        </li>
        <li>
          <h3>Name the import</h3>
          <input className="settings-input mig-name-input" value={name} maxLength={200} onChange={(e) => setName(e.target.value)} aria-label="Import name" />
        </li>
      </ol>
      {rejected.length > 0 && (
        <p className="form-error">
          <AlertTriangle size={14} /> Not recognized as an export: {rejected.join(', ')}. Use the CSV files of the searches above.
        </p>
      )}
      {create.isError && <p className="form-error">{(create.error as Error).message}</p>}
      <footer className="mig-footer">
        <button className="btn" onClick={() => navigate('/logs/import')}>
          Cancel
        </button>
        <button className="btn btn-primary" disabled={!assets.length || !name.trim() || create.isPending} onClick={() => create.mutate()}>
          {create.isPending ? 'Translating…' : 'Start Import'}
        </button>
      </footer>
    </div>
  );
}

// ================================================================ review

const KIND_ICON = { alert: Bell, dashboard: LayoutDashboard, search: Search } as const;

/** An import's assets: their translations, what's left to fix, and migration. */
export function ImportDetailPage() {
  const { id = '' } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['log-import', id], queryFn: () => getImport(id) });
  const [kind, setKind] = useState<'all' | ImportAsset['kind']>('all');
  const [status, setStatus] = useState<'' | ImportStatus>('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [open, setOpen] = useState<string | null>(null);
  const set = (d: LogImportDetail) => {
    qc.setQueryData(['log-import', id], d);
    void qc.invalidateQueries({ queryKey: ['log-imports'] });
  };
  const migrate = useMutation({ mutationFn: (ids: string[]) => migrateImport(id, ids), onSuccess: (d) => (set(d), setSelected(new Set())) });
  const remove = useMutation({ mutationFn: () => deleteImport(id), onSuccess: () => (void qc.invalidateQueries({ queryKey: ['log-imports'] }), navigate('/logs/import')) });
  const assets = useMemo(() => (q.data?.assets ?? []).filter((a) => (kind === 'all' || a.kind === kind) && (!status || a.status === status)), [q.data, kind, status]);
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const d = q.data;
  const migratable = (a: ImportAsset) => a.kind !== 'search' && a.status !== 'migrated' && a.status !== 'unsupported';
  const cards: [string, number, ImportStatus | ''][] = [
    ['Assets', d.counts.total, ''],
    ['Ready', d.counts.ready, 'ready'],
    ['Needs review', d.counts.partial, 'partial'],
    ['Not supported', d.counts.unsupported, 'unsupported'],
    ['Migrated', d.counts.migrated, 'migrated'],
    ['Failed', d.counts.failed, 'failed'],
  ];
  return (
    <div className="settings-pad mig-page">
      <Link to="/logs/import" className="mig-back">
        <ChevronLeft size={14} /> Back to Imports
      </Link>
      <div className="settings-title-row">
        <div>
          <h1 className="settings-title">{d.name}</h1>
          <p className="muted">
            Splunk · created {timeAgo(d.createdAt)}
            {d.createdBy ? ` by ${d.createdBy}` : ''} · {d.macros} macros
          </p>
        </div>
        <button className="btn" onClick={() => window.confirm(`Delete the import "${d.name}"? Monitors and dashboards already migrated stay.`) && remove.mutate()}>
          <Trash2 size={14} /> Delete
        </button>
      </div>
      <div className="mig-cards">
        {cards.map(([label, n, s]) => (
          <button key={label} className={`mig-card${status === s ? ' active' : ''}`} onClick={() => setStatus(s)}>
            <small>{label}</small>
            <b className={s ? `mig-text-${s}` : ''}>{n}</b>
          </button>
        ))}
      </div>
      <div className="mig-toolbar">
        <div className="btn-group">
          {(['all', 'alert', 'dashboard', 'search'] as const).map((k) => (
            <button key={k} className={`btn${kind === k ? ' btn-active' : ''}`} onClick={() => setKind(k)}>
              {k === 'all' ? 'All' : `${k[0].toUpperCase()}${k.slice(1)}s`}
            </button>
          ))}
        </div>
        <span className="muted">{assets.length} assets</span>
        <span className="mig-spacer" />
        <button className="btn" disabled={!selected.size || migrate.isPending} onClick={() => migrate.mutate([...selected])}>
          Migrate Selected ({selected.size})
        </button>
        <button className="btn btn-primary" disabled={!d.assets.some((a) => migratable(a) && a.status !== 'failed') || migrate.isPending} onClick={() => migrate.mutate([])}>
          {migrate.isPending ? 'Migrating…' : 'Migrate All'}
        </button>
      </div>
      {migrate.isError && <p className="form-error">{(migrate.error as Error).message}</p>}
      <table className="settings-table mig-assets">
        <thead>
          <tr>
            <th />
            <th />
            <th>NAME</th>
            <th>STATUS</th>
            <th>OBSERVECRAFT SEARCH</th>
            <th>RESULT</th>
          </tr>
        </thead>
        <tbody>
          {assets.map((a) => {
            const Icon = KIND_ICON[a.kind];
            const s = a.searches[0];
            return (
              <AssetRows key={a.id} a={a} open={open === a.id} onToggle={() => setOpen(open === a.id ? null : a.id)}>
                <td onClick={(e) => e.stopPropagation()}>
                  <input
                    type="checkbox"
                    aria-label={`Select ${a.title}`}
                    disabled={!migratable(a)}
                    checked={selected.has(a.id)}
                    onChange={(e) => {
                      const next = new Set(selected);
                      if (e.target.checked) next.add(a.id);
                      else next.delete(a.id);
                      setSelected(next);
                    }}
                  />
                </td>
                <td>{open === a.id ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</td>
                <td>
                  <span className="mig-name">
                    <Icon size={14} className="muted" /> <b>{a.title}</b>
                  </span>
                  {a.description && <div className="muted mig-desc">{a.description}</div>}
                </td>
                <td>
                  <StatusPill s={a.status} />
                </td>
                <td className="mono mig-query">{a.kind === 'dashboard' ? <span className="muted">{a.searches.length} panels</span> : s?.translation.query || '*'}</td>
                <td onClick={(e) => e.stopPropagation()}>
                  {a.targetId ? (
                    <Link to={a.targetKind === 'monitor' ? `/monitors/${a.targetId}` : `/dashboards/${a.targetId}`}>{a.targetKind === 'monitor' ? 'Monitor' : 'Dashboard'} ↗</Link>
                  ) : a.error ? (
                    <span className="form-error">{a.error}</span>
                  ) : a.kind === 'search' && s ? (
                    <Link to={explorerLink(s.translation)}>Open in Log Explorer</Link>
                  ) : (
                    <span className="muted">—</span>
                  )}
                </td>
              </AssetRows>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function AssetRows({ a, open, onToggle, children }: { a: ImportAsset; open: boolean; onToggle: () => void; children: ReactNode }) {
  return (
    <>
      <tr className="clickable" onClick={onToggle}>
        {children}
      </tr>
      {open && (
        <tr className="mig-detail-row">
          <td colSpan={6}>
            {a.issues.length > 0 && (
              <ul className="mig-warnings">
                {a.issues.map((i) => (
                  <li key={i}>
                    <AlertTriangle size={14} /> {i}
                  </li>
                ))}
              </ul>
            )}
            {a.alert && (
              <div className="mig-alert">
                <b>Monitor</b> count of matching logs{' '}
                {a.alert.comparator && a.alert.threshold != null
                  ? `${{ above: '>', above_or_equal: '≥', below: '<', below_or_equal: '≤' }[a.alert.comparator] ?? a.alert.comparator} ${a.alert.threshold}`
                  : '(no threshold)'}{' '}
                over the last {a.alert.windowMinutes} minutes
                {a.alert.recipients.length > 0 && <> · notifies {a.alert.recipients.join(', ')}</>}
                {a.alert.schedule && <span className="muted"> · was scheduled {a.alert.schedule}</span>}
              </div>
            )}
            {a.searches.map((s, i) => (
              <div key={i} className="mig-search-detail">
                {a.kind === 'dashboard' && <h4>{s.title}</h4>}
                <div className="mig-pair">
                  <label>SPL</label>
                  <code>{s.spl}</code>
                  <label>ObserveCraft</label>
                  <code>{s.translation.query || '*'}</code>
                  {s.translation.aggregation && (
                    <>
                      <label>Aggregation</label>
                      <span>{describeAggregation(s.translation.aggregation)}</span>
                    </>
                  )}
                </div>
                {s.translation.warnings.length > 0 && (
                  <ul className="mig-warnings">
                    {s.translation.warnings.map((w) => (
                      <li key={w}>
                        <AlertTriangle size={14} /> {w}
                      </li>
                    ))}
                  </ul>
                )}
                <div className="mig-links">
                  <Link to={explorerLink(s.translation)}>Open in Log Explorer</Link>
                  <Link to={`/logs/translate?q=${encodeURIComponent(s.spl)}`}>Edit in Translate</Link>
                </div>
              </div>
            ))}
          </td>
        </tr>
      )}
    </>
  );
}
