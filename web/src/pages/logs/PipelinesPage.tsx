import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowDown, ArrowUp, ChevronDown, ChevronRight, Copy, FlaskConical, Lock, Package, Pencil, Plus, Search, Trash2, X } from 'lucide-react';
import {
  createPipeline,
  deletePipeline,
  installLibraryPipeline,
  isSupported,
  listLibrary,
  listPipelines,
  processorLabel,
  reorderPipelines,
  setPipelineEnabled,
  updatePipeline,
  type LogPipeline,
  type LogPipelineInput,
  type NestedPipeline,
  type Processor,
} from '../../api/pipelines/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { useProfile } from '../settings/queries';
import { PipelineTestModal } from './PipelineTestModal';
import { ProcessorEditor } from './ProcessorEditor';

const KEY = ['log-pipelines'];
const LIBRARY_KEY = ['log-pipeline-library'];

/** Which processor the editor is open on: a list (top level, or a nested pipeline's) and an index (null = new). */
interface ProcessorTarget {
  pipelineId: string;
  parent: number | null;
  index: number | null;
}

const input = (p: LogPipeline, processors = p.processors): LogPipelineInput => ({ name: p.name, filter: p.filter, enabled: p.enabled, processors });

/** The processor list at `parent` (a nested pipeline's index) and the top-level list with it replaced. */
function listAt(p: LogPipeline, parent: number | null): Processor[] {
  return parent === null ? p.processors : ((p.processors[parent] as NestedPipeline).processors ?? []);
}
function replaceList(p: LogPipeline, parent: number | null, list: Processor[]): Processor[] {
  if (parent === null) return list;
  return p.processors.map((x, i) => (i === parent ? { ...(x as NestedPipeline), processors: list } : x));
}

/** One line describing what a processor does. */
export function processorSummary(p: Processor): string {
  const arrow = (from: string[] | string, to: string) => `${Array.isArray(from) ? from.join(', ') : from} → ${to}`;
  switch (p.type) {
    case 'grok-parser': {
      const rules = (p.grok?.matchRules ?? '').split('\n').filter((l) => l.trim() && !l.trim().startsWith('#')).length;
      return `${p.source || 'message'} · ${rules} rule${rules === 1 ? '' : 's'}`;
    }
    case 'status-remapper':
    case 'date-remapper':
    case 'service-remapper':
    case 'message-remapper':
    case 'trace-id-remapper':
      return arrow(p.sources, p.type.replace('-remapper', '').replace('trace-id', 'trace id'));
    case 'attribute-remapper':
      return arrow(
        p.sources.map((s) => (p.sourceType === 'tag' ? `tag ${s}` : s)),
        p.targetType === 'tag' ? `tag ${p.target}` : p.target,
      );
    case 'category-processor':
      return `${p.categories.length} categor${p.categories.length === 1 ? 'y' : 'ies'} → ${p.target}`;
    case 'string-builder-processor':
      return `"${p.template}" → ${p.target}`;
    case 'lookup-processor':
      return `${arrow(p.source, p.target)} (${p.lookupTable.filter((l) => l.trim()).length} entries)`;
    case 'arithmetic-processor':
      return `${p.expression} → ${p.target}`;
    case 'url-parser':
      return arrow(p.sources, p.target);
    case 'pipeline':
      return `${p.filter?.query || 'all logs'} · ${p.processors?.length ?? 0} processors`;
  }
  return '';
}

/** Log Configuration → Pipelines: parse and enrich logs before they are indexed. */
export function PipelinesPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const org = me.data?.org.id;
  const canEdit = me.data?.roles.some((r) => r.id === `${org}-admin` || r.id === `${org}-standard`) ?? false;
  const q = useQuery({ queryKey: KEY, queryFn: listPipelines });
  const [search, setSearch] = useState('');
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState<{ pipeline: LogPipeline | null; clone: boolean } | null>(null);
  const [target, setTarget] = useState<ProcessorTarget | null>(null);
  const [testing, setTesting] = useState<LogPipeline | null>(null);
  const [library, setLibrary] = useState(false);
  const refresh = () => (qc.invalidateQueries({ queryKey: KEY }), qc.invalidateQueries({ queryKey: LIBRARY_KEY }));
  const reorder = useMutation({ mutationFn: reorderPipelines, onSuccess: refresh });
  const remove = useMutation({ mutationFn: deletePipeline, onSuccess: refresh });
  const toggle = useMutation({ mutationFn: (p: LogPipeline) => setPipelineEnabled(p.id, !p.enabled), onSuccess: refresh });
  const saveProcessors = useMutation({ mutationFn: ({ p, processors }: { p: LogPipeline; processors: Processor[] }) => updatePipeline(p.id, input(p, processors)), onSuccess: refresh });

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const pipelines = q.data;
  const needle = search.trim().toLowerCase();
  const shown = pipelines.filter((p) => !needle || p.name.toLowerCase().includes(needle) || p.filter.toLowerCase().includes(needle));
  const move = (i: number, d: -1 | 1) => {
    const ids = pipelines.map((x) => x.id);
    [ids[i], ids[i + d]] = [ids[i + d], ids[i]];
    reorder.mutate(ids);
  };
  const expand = (id: string) =>
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const targetPipeline = target && pipelines.find((p) => p.id === target.pipelineId);
  const mutationError = reorder.error ?? remove.error ?? toggle.error ?? (target ? null : saveProcessors.error);

  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Pipelines</h1>
        <div className="pl-title-actions">
          <button className="btn" onClick={() => setLibrary(true)}>
            <Package size={14} /> Browse Integration Pipelines
          </button>
          {canEdit && (
            <button className="btn btn-primary" onClick={() => setEditing({ pipeline: null, clone: false })}>
              <Plus size={14} /> New Pipeline
            </button>
          )}
        </div>
      </div>
      <p className="muted">
        Every incoming log goes through each enabled pipeline whose filter matches, top to bottom, before it is routed to an <Link to="/logs/indexes">index</Link>. Processors parse the message (grok),
        set the official status, date, service and message, and add attributes, so index filters, searches and facets see the processed log. Live Tail and archives keep the log as it arrived.
      </p>
      <div className="pl-search">
        <Search size={14} className="muted" />
        <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Filter pipelines by name or query" />
      </div>
      {mutationError && <p className="form-error">{(mutationError as Error).message}</p>}
      {pipelines.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No pipelines: logs are indexed as they arrive. Create one, or install an integration’s pipeline.</p>
        </section>
      ) : (
        <div className="pl-list">
          {shown.map((p) => {
            const i = pipelines.indexOf(p);
            const open = expanded.has(p.id);
            const editable = canEdit && !p.readOnly;
            return (
              <section key={p.id} className={`pl-card${p.enabled ? '' : ' disabled'}`}>
                <div className="pl-row">
                  <button className="icon-plain" onClick={() => expand(p.id)} title={open ? 'Hide processors' : 'Show processors'} aria-expanded={open}>
                    {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                  </button>
                  <span className="muted pl-pos">{i + 1}</span>
                  <button
                    role="switch"
                    aria-checked={p.enabled}
                    aria-label={p.enabled ? `Disable ${p.name}` : `Enable ${p.name}`}
                    className={`switch${p.enabled ? ' on' : ''}`}
                    disabled={!canEdit || toggle.isPending}
                    onClick={() => toggle.mutate(p)}
                  />
                  <button className="pl-name" onClick={() => expand(p.id)}>
                    {p.name}
                  </button>
                  {p.readOnly && (
                    <span className="badge-soon pl-badge" title="Installed from an integration: clone it to make changes">
                      <Lock size={10} /> INTEGRATION
                    </span>
                  )}
                  <code className="pl-filter" title="Filter">
                    {p.filter || 'all logs'}
                  </code>
                  <span className="muted pl-count">
                    {p.processors.length} processor{p.processors.length === 1 ? '' : 's'}
                  </span>
                  <span className="ix-actions pl-actions">
                    <button className="icon-plain" title="Test with a sample log" onClick={() => setTesting(p)}>
                      <FlaskConical size={15} />
                    </button>
                    {canEdit && (
                      <>
                        <button className="icon-plain" title="Move up" disabled={i === 0 || reorder.isPending || !!needle} onClick={() => move(i, -1)}>
                          <ArrowUp size={15} />
                        </button>
                        <button className="icon-plain" title="Move down" disabled={i === pipelines.length - 1 || reorder.isPending || !!needle} onClick={() => move(i, 1)}>
                          <ArrowDown size={15} />
                        </button>
                        {editable && (
                          <button className="icon-plain" title="Edit" onClick={() => setEditing({ pipeline: p, clone: false })}>
                            <Pencil size={15} />
                          </button>
                        )}
                        <button className="icon-plain" title="Clone" onClick={() => setEditing({ pipeline: p, clone: true })}>
                          <Copy size={15} />
                        </button>
                        <button className="icon-plain" title="Delete" onClick={() => window.confirm(`Delete pipeline "${p.name}"? Logs are no longer processed by it.`) && remove.mutate(p.id)}>
                          <Trash2 size={15} />
                        </button>
                      </>
                    )}
                  </span>
                </div>
                {open && (
                  <ProcessorList
                    processors={p.processors}
                    parent={null}
                    editable={editable}
                    busy={saveProcessors.isPending}
                    onEdit={(parent, index) => (saveProcessors.reset(), setTarget({ pipelineId: p.id, parent, index }))}
                    onChange={(parent, list) => saveProcessors.mutate({ p, processors: replaceList(p, parent, list) })}
                  />
                )}
              </section>
            );
          })}
          {shown.length === 0 && <p className="muted">No pipeline matches “{search}”.</p>}
        </div>
      )}
      {editing && <PipelineModal pipeline={editing.pipeline} clone={editing.clone} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), void refresh())} />}
      {target && targetPipeline && (
        <ProcessorEditor
          processor={target.index === null ? null : listAt(targetPipeline, target.parent)[target.index]}
          nested={target.parent !== null}
          saving={saveProcessors.isPending}
          error={saveProcessors.error as Error | null}
          onClose={() => setTarget(null)}
          onSave={(proc) => {
            const list = [...listAt(targetPipeline, target.parent)];
            if (target.index === null) list.push(proc);
            else list[target.index] = proc;
            saveProcessors.mutate({ p: targetPipeline, processors: replaceList(targetPipeline, target.parent, list) }, { onSuccess: () => setTarget(null) });
          }}
        />
      )}
      {testing && <PipelineTestModal pipeline={testing} onClose={() => setTesting(null)} />}
      {library && <LibraryModal canEdit={canEdit} onClose={() => setLibrary(false)} onInstalled={() => void refresh()} />}
    </div>
  );
}

interface ListProps {
  processors: Processor[];
  parent: number | null;
  editable: boolean;
  busy: boolean;
  onEdit: (parent: number | null, index: number | null) => void;
  onChange: (parent: number | null, list: Processor[]) => void;
}

/** A pipeline's processors in order, with a nested pipeline's own processors under it. */
function ProcessorList({ processors, parent, editable, busy, onEdit, onChange }: ListProps) {
  const swap = (i: number, d: -1 | 1) => {
    const list = [...processors];
    [list[i], list[i + d]] = [list[i + d], list[i]];
    onChange(parent, list);
  };
  return (
    <div className={parent === null ? 'pl-processors' : 'pl-processors pl-nested'}>
      {processors.length === 0 && <p className="muted pl-empty">No processors yet.</p>}
      <ol>
        {processors.map((proc, i) => {
          const supported = isSupported(proc.type);
          const off = proc.enabled === false;
          return (
            <li key={i}>
              <div className={`pl-proc${off ? ' disabled' : ''}`}>
                <span className="muted pl-pos">{i + 1}</span>
                <span className="pl-proc-type">{processorLabel(proc.type)}</span>
                <span className="pl-proc-name">{proc.name || <span className="muted">unnamed</span>}</span>
                <span className="mono muted pl-proc-summary">{supported ? processorSummary(proc) : ''}</span>
                {!supported && (
                  <span className="badge-soon" title="This processor type isn't supported yet: it is kept but skipped">
                    SKIPPED
                  </span>
                )}
                {off && <span className="badge-soon">DISABLED</span>}
                {editable && (
                  <span className="ix-actions pl-actions">
                    <button className="icon-plain" title="Move up" disabled={i === 0 || busy} onClick={() => swap(i, -1)}>
                      <ArrowUp size={14} />
                    </button>
                    <button className="icon-plain" title="Move down" disabled={i === processors.length - 1 || busy} onClick={() => swap(i, 1)}>
                      <ArrowDown size={14} />
                    </button>
                    {supported && (
                      <button className="icon-plain" title="Edit" onClick={() => onEdit(parent, i)}>
                        <Pencil size={14} />
                      </button>
                    )}
                    <button
                      className="icon-plain"
                      title="Delete"
                      disabled={busy}
                      onClick={() =>
                        window.confirm(`Delete processor "${proc.name || processorLabel(proc.type)}"?`) &&
                        onChange(
                          parent,
                          processors.filter((_, j) => j !== i),
                        )
                      }
                    >
                      <Trash2 size={14} />
                    </button>
                  </span>
                )}
              </div>
              {proc.type === 'pipeline' && parent === null && (
                <ProcessorList
                  processors={proc.processors ?? []}
                  parent={i}
                  editable={editable}
                  busy={busy}
                  onEdit={onEdit}
                  onChange={(_, list) =>
                    onChange(
                      null,
                      processors.map((x, j) => (j === i ? { ...proc, processors: list } : x)),
                    )
                  }
                />
              )}
            </li>
          );
        })}
      </ol>
      {editable && (
        <button className="btn btn-sm pl-add-proc" onClick={() => onEdit(parent, null)}>
          <Plus size={13} /> {parent === null ? 'Add Processor' : 'Add Nested Processor'}
        </button>
      )}
    </div>
  );
}

function PipelineModal({ pipeline, clone, onClose, onSaved }: { pipeline: LogPipeline | null; clone: boolean; onClose: () => void; onSaved: () => void }) {
  const [name, setName] = useState(pipeline ? (clone ? `${pipeline.name} (copy)` : pipeline.name) : '');
  const [filter, setFilter] = useState(pipeline?.filter ?? '');
  const [enabled, setEnabled] = useState(pipeline?.enabled ?? true);
  const body: LogPipelineInput = { name, filter, enabled, processors: pipeline?.processors ?? [] };
  const save = useMutation({ mutationFn: () => (pipeline && !clone ? updatePipeline(pipeline.id, body) : createPipeline(body)), onSuccess: onSaved });
  const title = pipeline ? (clone ? `Clone ${pipeline.name}` : `Edit ${pipeline.name}`) : 'New Pipeline';
  return (
    <Modal onClose={onClose} label={title} width={640} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{title}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Filter — logs matching this search go through the pipeline
          <input className="mono" value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="all logs (or e.g. service:api-gateway source:nginx)" autoFocus={!pipeline} />
        </label>
        <label className="form-label">
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="api-gateway access logs" maxLength={100} required />
        </label>
        <label className="ar-check">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Enabled
        </label>
        {clone && pipeline && <p className="muted">The copy keeps the {pipeline.processors.length} processors and can be edited.</p>}
        {!pipeline && <p className="muted">Add processors once the pipeline is created.</p>}
        {save.error && <p className="form-error pl-errors">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {pipeline && !clone ? 'Save' : 'Create Pipeline'}
          </button>
        </div>
      </form>
    </Modal>
  );
}

/** Integration pipelines (from integrations-core) to install, read-only. */
function LibraryModal({ canEdit, onClose, onInstalled }: { canEdit: boolean; onClose: () => void; onInstalled: () => void }) {
  const q = useQuery({ queryKey: LIBRARY_KEY, queryFn: listLibrary });
  const [open, setOpen] = useState<string | null>(null);
  const install = useMutation({ mutationFn: installLibraryPipeline, onSuccess: onInstalled });
  return (
    <Modal onClose={onClose} label="Integration pipelines" width={860} height="auto">
      <div className="small-modal">
        <div className="modal-title-row">
          <h3>Integration Pipelines</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <p className="muted">
          Ready-made pipelines for the logs of common technologies, from the open-source integrations-core repository (BSD-3-Clause). Installed pipelines are read-only; clone one to change it. They
          apply to logs with the matching <code>source</code>.
        </p>
        {q.isPending && <Loading />}
        {q.isError && <ErrorState error={q.error} onRetry={() => q.refetch()} />}
        {install.error && <p className="form-error">{(install.error as Error).message}</p>}
        <div className="pl-list">
          {q.data?.map((p) => (
            <section key={p.id} className="pl-card">
              <div className="pl-row">
                <button className="icon-plain" onClick={() => setOpen(open === p.id ? null : p.id)} aria-expanded={open === p.id} title="Show processors">
                  {open === p.id ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                </button>
                <span className="pl-name">{p.name}</span>
                <code className="pl-filter">{p.filter}</code>
                <span className="muted pl-count">{p.processors.length} processors</span>
                <span className="pl-actions">
                  {p.installed ? (
                    <span className="badge-soon">INSTALLED</span>
                  ) : (
                    canEdit && (
                      <button className="btn btn-sm" disabled={install.isPending} onClick={() => install.mutate(p.id)}>
                        <Plus size={13} /> Install
                      </button>
                    )
                  )}
                </span>
              </div>
              {open === p.id && (
                <>
                  <ProcessorList processors={p.processors} parent={null} editable={false} busy={false} onEdit={() => {}} onChange={() => {}} />
                  <p className="muted pl-upstream">
                    From {p.upstream.repository} <span className="mono">{p.upstream.path}</span> at <span className="mono">{p.upstream.revision.slice(0, 10)}</span> ({p.upstream.license})
                  </p>
                </>
              )}
            </section>
          ))}
        </div>
      </div>
    </Modal>
  );
}
