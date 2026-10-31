import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowDown, ArrowUp, CheckCircle2, ChevronDown, ChevronRight, Cloud, Code, Lock, Package, Pencil, Plus, Puzzle, Search, Shuffle, Trash2, X } from 'lucide-react';
import {
  createPipeline,
  deletePipeline,
  installLibraryPipeline,
  isSupported,
  listLibrary,
  PROCESSOR_TYPES,
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
import { pipelinesUsage } from '../../api/logsConfig/client';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { useProfile } from '../settings/queries';
import { PipelineTestModal } from './PipelineTestModal';
import { ProcessorEditor } from './ProcessorEditor';
import { Avatar, Columns, ConfigTitle, FilterQuery, FlowStep, fmtDate, LabeledSelect, Num, SectionHead, TagChips, UsageBars } from './config/configUi';

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

const USAGE_KEY = ['log-pipelines-usage'];
const DAY = 86_400_000;

/** Log Configuration → Pipelines: parse and enrich logs before they are indexed. */
export function PipelinesPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const org = me.data?.org.id;
  const canEdit = me.data?.roles.some((r) => r.id === `${org}-admin` || r.id === `${org}-standard`) ?? false;
  const q = useQuery({ queryKey: KEY, queryFn: listPipelines });
  const ids = (q.data ?? []).map((p) => p.id);
  const usage = useQuery({ queryKey: [...USAGE_KEY, ids.join()], queryFn: () => pipelinesUsage(ids), enabled: ids.length > 0, staleTime: 60_000 });
  const [search, setSearch] = useState('');
  const [enabledF, setEnabledF] = useState('all');
  const [typeF, setTypeF] = useState('all');
  const [editedF, setEditedF] = useState('all');
  const [userF, setUserF] = useState('all');
  const [containsF, setContainsF] = useState('all');
  const [editableF, setEditableF] = useState('all');
  const [open, setOpen] = useState(true);
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
  const now = Date.now();
  const users = [...new Set(pipelines.map((p) => p.updatedBy).filter((u): u is string => !!u))].sort();
  const hasType = (list: Processor[], t: string): boolean => list.some((x) => x.type === t || (x.type === 'pipeline' && hasType(x.processors ?? [], t)));
  const shown = pipelines.filter(
    (p) =>
      (!needle || p.name.toLowerCase().includes(needle) || p.filter.toLowerCase().includes(needle) || p.processors.some((x) => (x.name ?? '').toLowerCase().includes(needle))) &&
      (enabledF === 'all' || String(p.enabled) === enabledF) &&
      (typeF === 'all' || (typeF === 'integration') === p.readOnly) &&
      (editableF === 'all' || String(!p.readOnly) === editableF) &&
      (userF === 'all' || p.updatedBy === userF) &&
      (containsF === 'all' || hasType(p.processors, containsF)) &&
      (editedF === 'all' || (editedF === 'older' ? now - p.updatedAt > 30 * DAY : now - p.updatedAt <= Number(editedF) * DAY)),
  );
  const filtered = shown.length !== pipelines.length;
  const move = (i: number, d: -1 | 1) => {
    const order = pipelines.map((x) => x.id);
    [order[i], order[i + d]] = [order[i + d], order[i]];
    reorder.mutate(order);
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
  const active = pipelines.filter((p) => p.enabled).length;
  const share = new Map((usage.data ?? []).map((u) => [u.id, u]));
  const template = '20px 24px minmax(160px, 1.4fr) 56px minmax(90px, 0.8fr) minmax(160px, 1.6fr) 28px 112px 30px 74px';

  return (
    <div className="lc-page">
      <ConfigTitle
        title="Pipelines"
        actions={
          <>
            <button className="btn" onClick={() => setLibrary(true)}>
              <Package size={14} /> Browse Pipeline Library
            </button>
            {canEdit && (
              <button className="btn btn-primary" onClick={() => setEditing({ pipeline: null, clone: false })}>
                <Plus size={14} /> New Pipeline
              </button>
            )}
          </>
        }
      />
      <div className="lc-search">
        <Search size={14} className="muted" />
        <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Filter pipelines" aria-label="Filter pipelines" />
        <span className="lc-code" title="Matches pipeline names, filters and processor names">
          <Code size={14} />
        </span>
      </div>
      <div className="lc-filters">
        <LabeledSelect
          label="Is pipeline enabled"
          value={enabledF}
          onChange={setEnabledF}
          options={[
            ['all', 'All'],
            ['true', 'Enabled'],
            ['false', 'Disabled'],
          ]}
        />
        <LabeledSelect
          label="Pipeline Type"
          value={typeF}
          onChange={setTypeF}
          options={[
            ['all', 'All'],
            ['custom', 'Custom'],
            ['integration', 'Integration'],
          ]}
        />
        <LabeledSelect
          label="Time of last edit"
          value={editedF}
          onChange={setEditedF}
          options={[
            ['all', 'All'],
            ['1', 'Past day'],
            ['7', 'Past week'],
            ['30', 'Past month'],
            ['older', 'Older than a month'],
          ]}
        />
        <LabeledSelect label="User who last edited" value={userF} onChange={setUserF} options={[['all', 'All'], ...users.map((u): [string, string] => [u, u])]} />
        <LabeledSelect label="Contains" value={containsF} onChange={setContainsF} options={[['all', 'All'], ...PROCESSOR_TYPES.map((t): [string, string] => [t.type, t.label])]} />
        <LabeledSelect
          label="Is pipeline editable"
          value={editableF}
          onChange={setEditableF}
          options={[
            ['all', 'All'],
            ['true', 'Editable'],
            ['false', 'Read-only'],
          ]}
        />
      </div>
      <div className="lc-flow">
        <div className="lc-flow-box">
          <FlowStep icon={<Cloud size={15} />} label="Ingest API" to="/logs/onboarding" right="Datadog Agent, OpenTelemetry, HTTP" />
          <FlowStep icon={<CheckCircle2 size={15} />} label="Preprocessing for JSON logs" to="/logs/attributes" right="Reserved attributes: date, host, service, status, message, trace id" />
          <FlowStep icon={<Shuffle size={15} />} label="Standard attributes" to="/logs/attributes" last />
        </div>
        <div className="lc-flow-link" />
      </div>
      {mutationError && <p className="form-error">{(mutationError as Error).message}</p>}
      <section className="lc-section">
        <SectionHead open={open} onToggle={() => setOpen((o) => !o)} label="PIPELINES">
          <span>
            <b>{active}</b> active
          </span>
          <span className="sep">|</span>
          <span>
            <b>{pipelines.length - active}</b> disabled
          </span>
          {filtered && <span className="muted">· {shown.length} shown</span>}
        </SectionHead>
        {open && (
          <>
            <Columns template={template} cols={['', '', 'Pipeline name', 'Usage', 'Tags', 'Filters', '', 'Last edited', 'By', '']} />
            {shown.map((p) => {
              const i = pipelines.indexOf(p);
              const isOpen = expanded.has(p.id);
              const editable = canEdit && !p.readOnly;
              const integration = p.source.startsWith('integration:') ? p.source.slice('integration:'.length) : null;
              const u = share.get(p.id);
              return (
                <div key={p.id}>
                  <div className={`lc-row${p.enabled ? '' : ' dim'}`} style={{ gridTemplateColumns: template }}>
                    <button className="icon-plain lc-keep" onClick={() => expand(p.id)} title={isOpen ? 'Hide processors' : 'Show processors'} aria-expanded={isOpen}>
                      {isOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
                    </button>
                    <Num n={i + 1} muted={p.readOnly || !p.enabled} />
                    <span className="lc-cell-name">
                      <button onClick={() => expand(p.id)} title={p.name}>
                        {p.name}
                      </button>
                      {p.readOnly && (
                        <span title="Installed from an integration: clone it to make changes">
                          <Puzzle size={13} className="muted" />
                        </span>
                      )}
                      <span className="muted" title="Processors">
                        ({p.processors.length})
                      </span>
                    </span>
                    <UsageBars share={p.enabled ? u?.share : 0} title={u ? `${(u.share * 100).toFixed(1)}% of the last hour's indexed logs match its filter` : undefined} />
                    <TagChips tags={p.tags} />
                    <FilterQuery query={p.filter} />
                    <span>{integration && hasBrandLogo(integration) ? <BrandLogo name={integration} size={20} title={integration} /> : null}</span>
                    <span className="muted lc-nowrap">{p.readOnly ? '' : fmtDate(p.updatedAt)}</span>
                    <span>{!p.readOnly && <Avatar name={p.updatedBy} />}</span>
                    <span className="lc-actions lc-keep">
                      <button
                        role="switch"
                        aria-checked={p.enabled}
                        aria-label={p.enabled ? `Disable ${p.name}` : `Enable ${p.name}`}
                        className={`switch${p.enabled ? ' on' : ''}`}
                        disabled={!canEdit || toggle.isPending}
                        onClick={() => toggle.mutate(p)}
                      />
                      <Menu
                        trigger={<span aria-label={`Actions of ${p.name}`}>⋯</span>}
                        triggerClassName="icon-plain"
                        items={[
                          { label: 'Test with a sample log', onClick: () => setTesting(p) },
                          ...(canEdit
                            ? [
                                { label: 'Move up', onClick: () => move(i, -1), disabled: i === 0 || reorder.isPending || filtered || !!needle },
                                { label: 'Move down', onClick: () => move(i, 1), disabled: i === pipelines.length - 1 || reorder.isPending || filtered || !!needle },
                                ...(editable ? [{ label: 'Edit', onClick: () => setEditing({ pipeline: p, clone: false }) }] : []),
                                { label: 'Clone', onClick: () => setEditing({ pipeline: p, clone: true }) },
                                { label: 'Delete', onClick: () => window.confirm(`Delete pipeline "${p.name}"? Logs are no longer processed by it.`) && remove.mutate(p.id) },
                              ]
                            : []),
                        ]}
                      />
                    </span>
                  </div>
                  {isOpen && (
                    <div className="lc-detail">
                      {p.readOnly && (
                        <p className="muted pl-readonly">
                          <Lock size={12} /> Installed from the {integration} integration: read-only, clone it to make changes.
                        </p>
                      )}
                      <ProcessorList
                        processors={p.processors}
                        parent={null}
                        editable={editable}
                        busy={saveProcessors.isPending}
                        onEdit={(parent, index) => (saveProcessors.reset(), setTarget({ pipelineId: p.id, parent, index }))}
                        onChange={(parent, list) => saveProcessors.mutate({ p, processors: replaceList(p, parent, list) })}
                      />
                    </div>
                  )}
                </div>
              );
            })}
            {pipelines.length === 0 && <p className="lc-empty">No pipelines: logs are indexed as they arrive. Create one, or install one from the library.</p>}
            {pipelines.length > 0 && shown.length === 0 && <p className="lc-empty">No pipeline matches these filters.</p>}
            <button className="lc-add-row" disabled={!canEdit} onClick={() => setEditing({ pipeline: null, clone: false })}>
              <Plus size={14} /> Add a new pipeline
            </button>
          </>
        )}
      </section>
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
