import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowDown, ArrowUp, Ban, ChevronDown, ChevronRight, ExternalLink, Pencil, Plus, Search, Trash2, Workflow, X } from 'lucide-react';
import { createIndex, deleteIndex, listIndexes, reorderIndexes, updateIndex, type ExclusionFilter, type LogIndex, type LogIndexInput } from '../../api/logIndexes/client';
import { getFlexSettings } from '../../api/logsConfig/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { compact } from '../../lib/format';
import { useProfile } from '../settings/queries';
import { Columns, ConfigTitle, FilterQuery, FlowStep, Num, SectionHead, TagChips } from './config/configUi';

const RETENTIONS = [1, 3, 7, 15, 30, 45, 60, 90, 180, 365, 730];
const FLEX_DAYS = [30, 60, 90, 180, 360, 450];
const KEY = ['log-indexes'];

/** Log Configuration → Indexes: route logs to indexes with their own retention, quota and exclusions. */
export function IndexesPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  const q = useQuery({ queryKey: KEY, queryFn: listIndexes, refetchInterval: 30_000 });
  const flex = useQuery({ queryKey: ['log-flex-settings'], queryFn: getFlexSettings });
  const [editing, setEditing] = useState<LogIndex | 'new' | null>(null);
  const [view, setView] = useState('all');
  const [search, setSearch] = useState('');
  const [open, setOpen] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: KEY });
  const reorder = useMutation({ mutationFn: reorderIndexes, onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteIndex, onSuccess: refresh });

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const { indexes, ingestedToday, maxRetentionDays, maxIndexes } = q.data;
  const indexedToday = indexes.reduce((n, i) => n + i.indexedToday, 0);
  const flexOn = flex.data?.enabled ?? false;
  const needle = search.trim().toLowerCase();
  const shown = indexes.filter(
    (ix) =>
      (!needle || ix.name.includes(needle) || ix.filter.toLowerCase().includes(needle)) &&
      (view === 'all' || (view === 'exclusions' && ix.exclusionFilters.length > 0) || (view === 'quota' && ix.dailyQuota != null) || (view === 'flex' && ix.flexRetentionDays != null)),
  );
  const move = (i: number, d: -1 | 1) => {
    const ids = indexes.map((x) => x.id);
    [ids[i], ids[i + d]] = [ids[i + d], ids[i]];
    reorder.mutate(ids);
  };
  const template = '20px 24px minmax(160px, 1fr) minmax(240px, 2.4fr) 150px 150px 110px';

  return (
    <div className="lc-page">
      <ConfigTitle
        title="Indexes"
        extra={<Link to="/logs/flex-controls">Flex Logs Controls</Link>}
        actions={
          isAdmin && (
            <button
              className="btn btn-primary"
              onClick={() => setEditing('new')}
              disabled={indexes.length >= maxIndexes}
              title={indexes.length >= maxIndexes ? `At most ${maxIndexes} indexes` : undefined}
            >
              <Plus size={14} /> New Index
            </button>
          )
        }
      />
      <div className="lc-view">
        <label className="lc-view-select">
          <span>View</span>
          <select value={view} onChange={(e) => setView(e.target.value)} aria-label="View">
            <option value="all">All indexes</option>
            <option value="exclusions">With exclusion filters</option>
            <option value="quota">With a daily quota</option>
            <option value="flex">With a flex tier</option>
          </select>
        </label>
        <div className="lc-search">
          <Search size={14} className="muted" />
          <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Filter Indexes" aria-label="Filter indexes" />
        </div>
      </div>
      <div className="lc-count">
        <b>{indexes.length}</b> indexes defined{' '}
        <span className="muted">
          · {compact(ingestedToday)} logs received today, {compact(indexedToday)} indexed · up to {maxRetentionDays} days standard retention
        </span>
      </div>
      <div className="lc-flow">
        <div className="lc-flow-box">
          <FlowStep icon={<Workflow size={15} />} label="Pipelines" to="/logs/pipelines" last />
        </div>
        <div className="lc-flow-link" />
      </div>
      {(reorder.error || remove.error) && <p className="form-error">{((reorder.error ?? remove.error) as Error).message}</p>}
      <section className="lc-section">
        <SectionHead open={open} onToggle={() => setOpen((o) => !o)} label="INDEXES">
          <span>
            <b>{indexes.length}</b> active
          </span>
        </SectionHead>
        {open && (
          <>
            <Columns template={template} cols={['', '', 'Indexes', 'Filters', 'Indexed today', 'Retention', '']} lock={false} />
            {shown.map((ix) => {
              const i = indexes.indexOf(ix);
              const pct = ix.dailyQuota ? Math.min(100, (ix.indexedToday / ix.dailyQuota) * 100) : null;
              const excl = ix.exclusionFilters.filter((e) => e.enabled).length;
              const isOpen = expanded === ix.id;
              return (
                <div key={ix.id}>
                  <div className="lc-row" style={{ gridTemplateColumns: template }}>
                    <button className="icon-plain" onClick={() => setExpanded(isOpen ? null : ix.id)} aria-expanded={isOpen} title="Details">
                      {isOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
                    </button>
                    <Num n={i + 1} />
                    <span className="lc-cell-name">
                      <Link className="mono" to={`/logs?query=${encodeURIComponent(`index:${ix.name}`)}`} title="Search this index">
                        {ix.name}
                      </Link>
                      <ExternalLink size={12} className="muted" />
                    </span>
                    <span className="lc-cell-name">
                      <FilterQuery query={ix.filter} />
                      {excl > 0 && (
                        <span className="lc-ex" title={ix.exclusionFilters.map((e) => `${e.name || e.query}: ${e.sampleRate}%`).join('\n')}>
                          <Ban size={12} /> {excl} exclusion filter{excl === 1 ? '' : 's'}
                        </span>
                      )}
                    </span>
                    <span title={ix.dailyQuota ? `Daily quota ${ix.dailyQuota.toLocaleString()} (resets ${String(ix.quotaResetHour).padStart(2, '0')}:00 UTC)` : 'No daily quota'}>
                      {compact(ix.indexedToday)}
                      {pct !== null && (
                        <span className="ix-quota">
                          <span className={`ix-quota-bar${pct >= 100 ? ' full' : pct >= 80 ? ' high' : ''}`}>
                            <i style={{ width: `${pct}%` }} />
                          </span>
                          <span className="muted">{pct >= 100 ? 'quota reached' : `of ${compact(ix.dailyQuota!)}`}</span>
                        </span>
                      )}
                    </span>
                    <span>
                      {ix.retentionDays} days
                      {ix.flexRetentionDays != null && (
                        <span
                          className={`lc-pill ${flexOn ? 'info' : 'muted'}`}
                          title={flexOn ? `Kept ${ix.flexRetentionDays} days in total with the flex tier` : 'Flex Logs is off: the flex tier is not applied'}
                        >
                          {' '}
                          +flex {ix.flexRetentionDays}d
                        </span>
                      )}
                    </span>
                    <span className="lc-actions">
                      {isAdmin && (
                        <>
                          <button className="icon-plain" title="Move up" disabled={i === 0 || reorder.isPending || shown.length !== indexes.length} onClick={() => move(i, -1)}>
                            <ArrowUp size={15} />
                          </button>
                          <button className="icon-plain" title="Move down" disabled={i === indexes.length - 1 || reorder.isPending || shown.length !== indexes.length} onClick={() => move(i, 1)}>
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
                        </>
                      )}
                    </span>
                  </div>
                  {isOpen && (
                    <div className="lc-detail">
                      <dl className="lc-kv">
                        <dt>Filter</dt>
                        <dd className="mono">{ix.filter || '* (every log)'}</dd>
                        <dt>Retention</dt>
                        <dd>
                          {ix.retentionDays} days standard
                          {ix.flexRetentionDays != null && `, ${ix.flexRetentionDays} days in total with the flex tier${flexOn ? '' : ' (Flex Logs is off)'}`}
                        </dd>
                        <dt>Daily quota</dt>
                        <dd>{ix.dailyQuota ? `${ix.dailyQuota.toLocaleString()} logs, resets at ${String(ix.quotaResetHour).padStart(2, '0')}:00 UTC` : 'Unlimited'}</dd>
                        <dt>Exclusion filters</dt>
                        <dd>
                          {ix.exclusionFilters.length === 0
                            ? 'None'
                            : ix.exclusionFilters.map((e) => (
                                <div key={e.name + e.query} className={e.enabled ? '' : 'muted'}>
                                  {e.name && <b>{e.name}: </b>}
                                  <code>{e.query || '*'}</code> excluded at {e.sampleRate}%{e.sampleAttribute && <> by {e.sampleAttribute}</>}
                                  {!e.enabled && ' (disabled)'}
                                </div>
                              ))}
                        </dd>
                        {ix.tags.length > 0 && (
                          <>
                            <dt>Tags</dt>
                            <dd>
                              <TagChips tags={ix.tags} max={10} />
                            </dd>
                          </>
                        )}
                      </dl>
                    </div>
                  )}
                </div>
              );
            })}
            {indexes.length === 0 && <p className="lc-empty">No indexes: logs are only available in Live Tail and archives.</p>}
            {indexes.length > 0 && shown.length === 0 && <p className="lc-empty">No index matches.</p>}
          </>
        )}
      </section>
      <p className="muted lc-note">
        Each incoming log goes to the <b>first</b> index whose filter matches. Logs that no index keeps (no match, excluded, or over quota) still reach <Link to="/logs/livetail">Live Tail</Link>,{' '}
        <Link to="/logs/archives">archives</Link> and custom destinations. Search one index with <code>index:&lt;name&gt;</code>.
      </p>
      {editing && (
        <IndexModal index={editing === 'new' ? null : editing} maxRetention={maxRetentionDays} flexOn={flexOn} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), void refresh())} />
      )}
    </div>
  );
}

function IndexModal({ index, maxRetention, flexOn, onClose, onSaved }: { index: LogIndex | null; maxRetention: number; flexOn: boolean; onClose: () => void; onSaved: () => void }) {
  const [name, setName] = useState(index?.name ?? '');
  const [filter, setFilter] = useState(index?.filter ?? '');
  const [retention, setRetention] = useState(index?.retentionDays ?? Math.min(15, maxRetention));
  const [quota, setQuota] = useState(index?.dailyQuota ? String(index.dailyQuota) : '');
  const [resetHour, setResetHour] = useState(index?.quotaResetHour ?? 0);
  const [exclusions, setExclusions] = useState<ExclusionFilter[]>(index?.exclusionFilters ?? []);
  const [flexDays, setFlexDays] = useState<number | null>(index?.flexRetentionDays ?? null);
  const [tags, setTags] = useState((index?.tags ?? []).join(', '));
  const input: LogIndexInput = {
    name,
    filter,
    retentionDays: retention,
    dailyQuota: quota.trim() ? Number(quota) : null,
    quotaResetHour: resetHour,
    exclusionFilters: exclusions,
    flexRetentionDays: flexDays,
    tags: tags
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean),
  };
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
        <div className="ar-grid">
          <label className="form-label">
            Flex tier — total days in the index
            <select value={flexDays ?? ''} onChange={(e) => setFlexDays(e.target.value ? Number(e.target.value) : null)}>
              <option value="">No flex tier</option>
              {FLEX_DAYS.filter((d) => d >= retention).map((d) => (
                <option key={d} value={d}>
                  {d} days
                </option>
              ))}
            </select>
          </label>
          <label className="form-label">
            Tags (key:value, comma-separated)
            <input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="team:checkout" />
          </label>
        </div>
        {flexDays != null && !flexOn && (
          <p className="muted">
            Flex Logs is off for this organization: turn it on in <Link to="/logs/flex-controls">Flex Logs Controls</Link> for this index to keep logs {flexDays} days.
          </p>
        )}
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
