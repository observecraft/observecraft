import { useState } from 'react';
import { ArrowDown, ArrowUp, CheckCircle2, ChevronDown, Filter, MoreVertical, Plus, Search, X, XCircle } from 'lucide-react';
import type { RetentionFilter, RetentionFilterInput } from '../../../api/apmSettings/client';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Menu } from '../../../components/ui/Dropdown';
import { Modal } from '../../../components/ui/Modal';
import { UserAvatar } from '../../../components/ui/UserAvatar';
import { big } from './format';
import { useDeleteRetentionFilter, useReorderRetentionFilters, useRetentionFilters, useSaveRetentionFilter } from './queries';

const ratePct = (r: number) => `${+(r * 100).toFixed(1)}%`;
const edited = (s: number) =>
  s ? new Date(s * 1000).toLocaleString(undefined, { month: 'short', day: '2-digit', year: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false }).replace(',', '') : '';

/** The org's mark for the default filters, or the editor's avatar. */
function Editor({ f }: { f: RetentionFilter }) {
  if (f.isDefault) return <img src="/logo.svg" alt="ObserveCraft" title="Default filter" width={22} height={22} />;
  return f.modifiedBy ? <UserAvatar id={f.modifiedBy} name={f.modifiedBy} size={22} /> : null;
}

/** APM → Settings → Retention Filters: which spans indexing keeps, in execution order. */
export function RetentionFiltersPage() {
  const { range } = useTime();
  const q = useRetentionFilters(range);
  const reorder = useReorderRetentionFilters();
  const save = useSaveRetentionFilter();
  const remove = useDeleteRetentionFilter();
  const [search, setSearch] = useState('');
  const [enabled, setEnabled] = useState('');
  const [mine, setMine] = useState(false);
  const [editing, setEditing] = useState<RetentionFilter | 'new' | null>(null);
  const [collapsed, setCollapsed] = useState(false);
  const all = q.data?.filters ?? [];
  const rows = all.filter(
    (f) =>
      (!search.trim() || `${f.name} ${f.query}`.toLowerCase().includes(search.trim().toLowerCase())) && (!enabled || String(f.enabled) === enabled) && (!mine || (!f.isDefault && f.createdBy !== '')),
  );
  const move = (i: number, d: -1 | 1) => {
    const ids = all.map((f) => f.id);
    const j = i + d;
    if (j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j], ids[i]];
    reorder.mutate(ids);
  };
  const toggle = (f: RetentionFilter) => save.mutate({ id: f.id, input: { name: f.name, query: f.query, rate: f.rate, traceRate: f.traceRate, enabled: !f.enabled } });
  const err = (reorder.error ?? save.error ?? remove.error) as Error | null;
  return (
    <div className="settings-pad aps-retention">
      <div className="aps-title-row">
        <div>
          <h1 className="aps-h1">Retention Filters</h1>
          <p className="muted">Choose which spans and traces are kept for indexed search and analytics.</p>
        </div>
        <div className="aps-title-actions">
          <TimePicker compact />
        </div>
      </div>
      <div className="aps-filters">
        <label className="aps-toggle aps-boxed">
          My Filters
          <input type="checkbox" checked={mine} onChange={(e) => setMine(e.target.checked)} />
          <span className="aps-toggle-track" />
        </label>
        <label className="aps-labeled">
          <span>Is Enabled</span>
          <select value={enabled} onChange={(e) => setEnabled(e.target.value)}>
            <option value="">All</option>
            <option value="true">Enabled</option>
            <option value="false">Disabled</option>
          </select>
        </label>
      </div>
      <div className="aps-search-row">
        <label className="aps-search aps-grow">
          <Search size={14} />
          <input placeholder="Search retention filters" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <button className="btn btn-primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> Add Retention Filter
        </button>
      </div>
      {err && <p className="form-error">{err.message}</p>}
      {q.isLoading ? (
        <Loading />
      ) : q.isError || !q.data ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <p className="aps-count">
            <b>{all.length}</b> retention filters <span className="aps-sep" /> <span className="aps-link-num">{big(q.data.spansIndexed)}</span> searchable spans
          </p>
          <section className="aps-group">
            <button className="aps-group-head" onClick={() => setCollapsed((c) => !c)}>
              <b>Retention Filters</b>
              <span>
                <b>{all.length}</b> retention filters
              </span>
              <span className="aps-sep" />
              <span>
                <span className="aps-link-num">{big(q.data.spansIndexed)}</span> searchable spans
              </span>
              <ChevronDown size={16} className={`aps-group-chev${collapsed ? ' rot' : ''}`} />
            </button>
            {!collapsed && (
              <div className="aps-table-wrap">
                <table className="aps-table aps-rf-table">
                  <thead>
                    <tr>
                      <th>ORDER</th>
                      <th>NAME</th>
                      <th>FILTER QUERY</th>
                      <th className="num">SPAN RATE</th>
                      <th className="num">TRACE RATE</th>
                      <th className="num">TOTAL SPANS INDEXED</th>
                      <th className="aps-center">ENABLED</th>
                      <th className="num">LAST EDITED</th>
                      <th />
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((f) => {
                      const i = all.findIndex((x) => x.id === f.id);
                      return (
                        <tr key={f.id} className={f.enabled ? '' : 'aps-off'}>
                          <td>
                            <span className="aps-order">
                              <span className="aps-order-num">{i + 1}</span>
                              <span className="aps-order-move">
                                <button className="icon-plain" title="Move up" disabled={i === 0 || reorder.isPending} onClick={() => move(i, -1)}>
                                  <ArrowUp size={12} />
                                </button>
                                <button className="icon-plain" title="Move down" disabled={i === all.length - 1 || reorder.isPending} onClick={() => move(i, 1)}>
                                  <ArrowDown size={12} />
                                </button>
                              </span>
                            </span>
                          </td>
                          <td>
                            <button className="link-btn aps-rf-name" onClick={() => setEditing(f)}>
                              {f.name}
                            </button>
                          </td>
                          <td className="mono aps-query" title={f.query}>
                            <Filter size={12} /> {f.query}
                          </td>
                          <td className="num">{ratePct(f.rate)}</td>
                          <td className="num">{ratePct(f.traceRate)}</td>
                          <td className="num" title={`${f.spansMatched.toLocaleString()} spans matched first by this filter`}>
                            {big(f.spansIndexed)}
                          </td>
                          <td className="aps-center">
                            <button className="icon-plain" onClick={() => toggle(f)} title={f.enabled ? 'Disable' : 'Enable'}>
                              {f.enabled ? <CheckCircle2 size={17} className="aps-ok" /> : <XCircle size={17} className="aps-grey" />}
                            </button>
                          </td>
                          <td className="num nowrap">{edited(f.modifiedAt)}</td>
                          <td>
                            <Editor f={f} />
                          </td>
                          <td>
                            <Menu
                              triggerClassName="icon-plain"
                              trigger={<MoreVertical size={15} />}
                              items={[
                                { label: 'Edit', onClick: () => setEditing(f) },
                                { label: f.enabled ? 'Disable' : 'Enable', onClick: () => toggle(f) },
                                { label: 'Delete', disabled: f.isDefault, onClick: () => window.confirm(`Delete the retention filter "${f.name}"?`) && remove.mutate(f.id) },
                              ]}
                            />
                          </td>
                        </tr>
                      );
                    })}
                    {rows.length === 0 && (
                      <tr>
                        <td colSpan={10} className="muted aps-center">
                          No retention filters match.
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            )}
          </section>
          <p className="muted aps-small">
            Spans go through the enabled filters in order; the first filter a span matches decides whether it is kept, at that filter's rate. Every ingested span stays searchable for the org's span
            retention: the counts show what the filters would keep.
          </p>
        </>
      )}
      {editing && <FilterModal filter={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function FilterModal({ filter, onClose }: { filter: RetentionFilter | null; onClose: () => void }) {
  const save = useSaveRetentionFilter();
  const [form, setForm] = useState<RetentionFilterInput>({
    name: filter?.name ?? '',
    query: filter?.query ?? '',
    rate: filter?.rate ?? 1,
    traceRate: filter?.traceRate ?? 0,
    enabled: filter?.enabled ?? true,
  });
  const set = (p: Partial<RetentionFilterInput>) => setForm((f) => ({ ...f, ...p }));
  return (
    <Modal onClose={onClose} label={filter ? 'Edit retention filter' : 'New retention filter'} width={640} height="auto">
      <form
        className="small-modal aps-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate({ id: filter?.id, input: form }, { onSuccess: onClose });
        }}
      >
        <div className="modal-title-row">
          <h3>{filter ? `Edit ${filter.name}` : 'Add a Retention Filter'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="aps-field">
          Name
          <input value={form.name} onChange={(e) => set({ name: e.target.value })} placeholder="Slow checkout requests" autoFocus />
        </label>
        <label className="aps-field">
          Filter query (span search syntax)
          <input className="mono" value={form.query} onChange={(e) => set({ query: e.target.value })} placeholder='service:storefront-api resource_name:"POST /checkout"' />
        </label>
        <div className="aps-form-grid">
          <label className="aps-field">
            Span rate (%)
            <input type="number" min={0} max={100} step={0.1} value={+(form.rate * 100).toFixed(2)} onChange={(e) => set({ rate: Number(e.target.value) / 100 })} />
          </label>
          <label className="aps-field">
            Trace rate (%)
            <input type="number" min={0} max={100} step={0.1} value={+((form.traceRate ?? 0) * 100).toFixed(2)} onChange={(e) => set({ traceRate: Number(e.target.value) / 100 })} />
          </label>
        </div>
        <p className="muted aps-small">The span rate keeps that share of the matching spans; the trace rate keeps the whole traces of that share.</p>
        <label className="aps-check">
          <input type="checkbox" checked={form.enabled} onChange={(e) => set({ enabled: e.target.checked })} /> Enabled
        </label>
        {filter?.isDefault && <p className="muted aps-small">A default filter: its query and rates can change, but it can't be deleted.</p>}
        {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!form.name.trim() || save.isPending}>
            {filter ? 'Save' : 'Create'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
