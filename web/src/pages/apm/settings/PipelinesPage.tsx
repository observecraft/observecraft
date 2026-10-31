import { Fragment, useState } from 'react';
import { ChevronRight, Filter, Info, Pencil, Plus, Search, Trash2, X } from 'lucide-react';
import type { SpanPipeline, SpanPipelineInput, SpanProcessor } from '../../../api/apmSettings/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { timeAgo } from '../../../lib/format';
import { useDeleteSpanPipeline, useSaveSpanPipeline, useSpanPipelines } from './queries';

const describe = (p: SpanProcessor) =>
  p.type === 'remapper' ? `${(p.sources ?? []).join(', ')} → ${p.target}` : `${p.target} from ${(p.categories ?? []).length} categor${(p.categories ?? []).length === 1 ? 'y' : 'ies'}`;

/** APM → Settings → Pipelines: processors on span attributes. */
export function SpanPipelinesPage() {
  const q = useSpanPipelines();
  const save = useSaveSpanPipeline();
  const remove = useDeleteSpanPipeline();
  const [search, setSearch] = useState('');
  const [enabled, setEnabled] = useState('');
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState<SpanPipeline | 'new' | null>(null);
  const all = q.data ?? [];
  const needle = search.trim().toLowerCase();
  const rows = all.filter(
    (p) => (!enabled || String(p.enabled) === enabled) && (!needle || `${p.name} ${p.query} ${p.processors.map((x) => `${x.name} ${x.target}`).join(' ')}`.toLowerCase().includes(needle)),
  );
  const toggle = (p: SpanPipeline) => save.mutate({ id: p.id, input: { name: p.name, query: p.query, enabled: !p.enabled, processors: p.processors } });
  return (
    <div className="settings-pad aps-pipelines">
      <h1 className="aps-h1">Pipelines</h1>
      <p className="muted">Transform, normalize and enrich span attributes after ingestion, without changing application code.</p>
      <div className="aps-notice">
        <Info size={15} /> Span pipelines are saved and listed; they are not applied to incoming spans yet.
      </div>
      <label className="aps-labeled aps-fit">
        <span>Is Pipeline Enabled</span>
        <select value={enabled} onChange={(e) => setEnabled(e.target.value)}>
          <option value="">All</option>
          <option value="true">Enabled</option>
          <option value="false">Disabled</option>
        </select>
      </label>
      <div className="aps-search-row">
        <label className="aps-search aps-grow">
          <Search size={14} />
          <input placeholder="Search pipeline or processor" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <button className="btn btn-primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> Add Pipeline
        </button>
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <p className="aps-count">
            <b>{all.filter((p) => p.enabled).length}</b> active pipelines
          </p>
          <section className="aps-group">
            <div className="aps-group-head aps-static">
              <b>Pipelines and Processors</b>
            </div>
            <table className="aps-table aps-pl-table">
              <thead>
                <tr>
                  <th style={{ width: 28 }} />
                  <th>NAME</th>
                  <th>FILTER QUERY</th>
                  <th className="aps-center">ENABLED</th>
                  <th className="num">LAST EDITED</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {rows.map((p) => {
                  const expanded = open.has(p.id);
                  return (
                    <Fragment key={p.id}>
                      <tr className={p.enabled ? '' : 'aps-off'}>
                        <td>
                          <button
                            className="icon-plain"
                            title={expanded ? 'Hide processors' : 'Show processors'}
                            onClick={() => setOpen((s) => (s.has(p.id) ? new Set([...s].filter((x) => x !== p.id)) : new Set([...s, p.id])))}
                          >
                            <ChevronRight size={14} className={expanded ? 'rot90' : ''} />
                          </button>
                        </td>
                        <td>
                          <b>{p.name}</b> <span className="muted">· {p.processors.length} processors</span>
                        </td>
                        <td className="mono aps-query">
                          <Filter size={12} /> {p.query || '*'}
                        </td>
                        <td className="aps-center">
                          <label className="aps-toggle">
                            <input type="checkbox" checked={p.enabled} onChange={() => toggle(p)} aria-label={`Enable ${p.name}`} />
                            <span className="aps-toggle-track" />
                          </label>
                        </td>
                        <td className="num muted nowrap" title={`${p.updatedBy} · ${new Date(p.updatedAt).toLocaleString()}`}>
                          {timeAgo(p.updatedAt)}
                        </td>
                        <td className="aps-row-actions">
                          <button className="icon-plain" title="Edit" onClick={() => setEditing(p)}>
                            <Pencil size={14} />
                          </button>
                          <button className="icon-plain" title="Delete" onClick={() => window.confirm(`Delete the pipeline "${p.name}"?`) && remove.mutate(p.id)}>
                            <Trash2 size={14} />
                          </button>
                        </td>
                      </tr>
                      {expanded &&
                        p.processors.map((x, i) => (
                          <tr key={`${p.id}-${i}`} className="aps-processor-row">
                            <td />
                            <td colSpan={5}>
                              <span className="aps-chip">{x.type.toUpperCase()}</span> <b>{x.name || describe(x)}</b> <span className="muted mono">{describe(x)}</span>
                            </td>
                          </tr>
                        ))}
                    </Fragment>
                  );
                })}
                {rows.length === 0 && (
                  <tr>
                    <td colSpan={6}>
                      <div className="aps-empty">
                        <Search size={44} className="aps-empty-icon" />
                        <h4>No matching results found</h4>
                      </div>
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
            <button className="aps-add-row" onClick={() => setEditing('new')}>
              <Plus size={15} /> Add a New Pipeline
            </button>
          </section>
        </>
      )}
      {editing && <PipelineModal pipeline={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

const blank = (type: string): SpanProcessor => ({ type, name: '', sources: [], target: '', preserveSource: true, categories: type === 'category' ? [{ name: '', query: '' }] : [] });

function PipelineModal({ pipeline, onClose }: { pipeline: SpanPipeline | null; onClose: () => void }) {
  const save = useSaveSpanPipeline();
  const [form, setForm] = useState<SpanPipelineInput>({ name: pipeline?.name ?? '', query: pipeline?.query ?? '', enabled: pipeline?.enabled ?? true, processors: pipeline?.processors ?? [] });
  const procs = form.processors ?? [];
  const setProc = (i: number, p: Partial<SpanProcessor>) => setForm((f) => ({ ...f, processors: procs.map((x, j) => (j === i ? { ...x, ...p } : x)) }));
  return (
    <Modal onClose={onClose} label={pipeline ? 'Edit pipeline' : 'New pipeline'} width={720} height="auto">
      <form
        className="small-modal aps-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate({ id: pipeline?.id, input: form }, { onSuccess: onClose });
        }}
      >
        <div className="modal-title-row">
          <h3>{pipeline ? `Edit ${pipeline.name}` : 'New Pipeline'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="aps-field">
          Name
          <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="Normalize HTTP routes" autoFocus />
        </label>
        <label className="aps-field">
          Filter query (spans the pipeline processes)
          <input className="mono" value={form.query ?? ''} onChange={(e) => setForm({ ...form, query: e.target.value })} placeholder="service:storefront-api" />
        </label>
        <label className="aps-check">
          <input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} /> Enabled
        </label>
        <div className="aps-field">
          Processors
          {procs.map((p, i) => (
            <div key={i} className="aps-proc">
              <div className="aps-proc-head">
                <select value={p.type} onChange={(e) => setProc(i, blank(e.target.value))} aria-label="Processor type">
                  <option value="remapper">Remapper</option>
                  <option value="category">Category</option>
                </select>
                <input value={p.name ?? ''} onChange={(e) => setProc(i, { name: e.target.value })} placeholder="Processor name" aria-label="Processor name" />
                <button type="button" className="icon-plain" title="Remove" onClick={() => setForm({ ...form, processors: procs.filter((_, j) => j !== i) })}>
                  <X size={14} />
                </button>
              </div>
              {p.type === 'remapper' ? (
                <div className="aps-group-by">
                  <input
                    className="mono"
                    value={(p.sources ?? []).join(', ')}
                    onChange={(e) =>
                      setProc(i, {
                        sources: e.target.value
                          .split(',')
                          .map((s) => s.trim())
                          .filter(Boolean),
                      })
                    }
                    placeholder="http.target, http.url_details.path"
                    aria-label="Source attributes"
                  />
                  <span className="muted">→</span>
                  <input className="mono" value={p.target ?? ''} onChange={(e) => setProc(i, { target: e.target.value })} placeholder="http.route" aria-label="Target attribute" />
                </div>
              ) : (
                <>
                  {(p.categories ?? []).map((c, k) => (
                    <div key={k} className="aps-group-by">
                      <input
                        value={c.name}
                        onChange={(e) => setProc(i, { categories: (p.categories ?? []).map((x, m) => (m === k ? { ...x, name: e.target.value } : x)) })}
                        placeholder="Value"
                        aria-label="Category"
                      />
                      <span className="muted">when</span>
                      <input
                        className="mono"
                        value={c.query}
                        onChange={(e) => setProc(i, { categories: (p.categories ?? []).map((x, m) => (m === k ? { ...x, query: e.target.value } : x)) })}
                        placeholder="@http.status_code:5*"
                        aria-label="Query"
                      />
                    </div>
                  ))}
                  <div className="aps-group-by">
                    <span className="muted">Write to</span>
                    <input className="mono" value={p.target ?? ''} onChange={(e) => setProc(i, { target: e.target.value })} placeholder="http.status_category" aria-label="Target attribute" />
                    <button type="button" className="link-btn" onClick={() => setProc(i, { categories: [...(p.categories ?? []), { name: '', query: '' }] })}>
                      <Plus size={13} /> Category
                    </button>
                  </div>
                </>
              )}
            </div>
          ))}
          <button type="button" className="link-btn aps-add-group" onClick={() => setForm({ ...form, processors: [...procs, blank('remapper')] })}>
            <Plus size={13} /> Add a processor
          </button>
        </div>
        {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!form.name.trim() || save.isPending}>
            {pipeline ? 'Save' : 'Create'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
