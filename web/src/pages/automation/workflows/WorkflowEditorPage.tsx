import { useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { AlertTriangle, ArrowLeft, Play, Settings, Trash2 } from 'lucide-react';
import { allSteps, freeId, insertStep, removeStep, replaceStep, specError, type BlockRef } from '../../../api/workflows/logic';
import type { ActionValues } from '../../../api/portalActions/types';
import type { Workflow, WorkflowStep, WorkflowStepKind } from '../../../api/workflows/types';
import { Modal } from '../../../components/ui/Modal';
import { ActionForm, initialValues } from '../../portal/actions/common';
import { useDeleteWorkflow, usePublishWorkflow, useRunWorkflow, useSaveWorkflow, useUnpublishWorkflow, useWorkflow } from '../queries';
import { Canvas } from './Canvas';
import { TRIGGER_ID } from './canvasLayout';
import { CatalogDrawer } from './CatalogDrawer';
import { ID_BASE, starterStep } from './catalog';
import { changed, fromWorkflow, toInput, type Draft } from './draft';
import { RunHistory } from './RunHistory';
import { SettingsPanel } from './SettingsPanel';
import { StepPanel } from './StepPanel';
import { TriggerPanel } from './TriggerPanel';
import './workflows.css';

type Panel = { kind: 'node'; id: string } | { kind: 'settings' } | null;

function RunDialog({
  workflow,
  draft,
  onClose,
  onRun,
  pending,
  error,
}: {
  workflow: Workflow;
  draft: Draft;
  onClose: () => void;
  onRun: (v: ActionValues) => void;
  pending: boolean;
  error: Error | null;
}) {
  const [values, setValues] = useState<ActionValues>(() => initialValues(draft.spec.inputs));
  return (
    <Modal onClose={onClose} label="Run the workflow" width={520} height="auto">
      <div className="wfe-dialog">
        <h3>Run {workflow.name}</h3>
        <p className="wf-muted">The saved draft runs now, as you.</p>
        <ActionForm inputs={draft.spec.inputs} values={values} onChange={setValues} />
        {error && <div className="wf-error">{error.message}</div>}
        <div className="wfe-dialog-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={pending} onClick={() => onRun(values)}>
            <Play size={13} /> Run
          </button>
        </div>
      </div>
    </Modal>
  );
}

function Editor({ workflow }: { workflow: Workflow }) {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const [saved, setSaved] = useState<Draft>(() => fromWorkflow(workflow));
  const [draft, setDraft] = useState<Draft>(saved);
  const initialStep = params.get('step');
  const [panel, setPanel] = useState<Panel>(() => (initialStep && initialStep !== 'catalog' ? { kind: 'node', id: initialStep === 'trigger' ? TRIGGER_ID : initialStep } : null));
  const [adding, setAdding] = useState<{ ref: BlockRef; index: number } | null>(() =>
    initialStep === 'catalog' ? { ref: { parent: null, branch: 'steps' }, index: workflow.spec.steps.length } : null,
  );
  const [runOpen, setRunOpen] = useState(false);
  const tab = params.get('run') !== null || params.get('tab') === 'runs' ? 'runs' : 'editor';
  const runId = params.get('run') ? Number(params.get('run')) : null;
  const save = useSaveWorkflow();
  const publish = usePublishWorkflow();
  const unpublish = useUnpublishWorkflow();
  const run = useRunWorkflow(workflow.id);
  const del = useDeleteWorkflow();
  const dirty = changed(draft, saved);
  const problem = useMemo(() => specError(draft.spec), [draft.spec]);
  const busy = save.isPending || publish.isPending || unpublish.isPending || run.isPending;
  const error = (save.error ?? publish.error ?? unpublish.error ?? del.error) as Error | null;

  useEffect(() => {
    if (initialStep) {
      const next = new URLSearchParams(params);
      next.delete('step');
      setParams(next, { replace: true });
    }
    // Only on the first render: the query parameter only says what to open.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!dirty) return;
    const warn = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty]);

  const persist = (then?: (w: Workflow) => void) => {
    if (!dirty) return then?.(workflow);
    save.mutate(
      { id: workflow.id, input: toInput(draft) },
      {
        onSuccess: (w) => {
          const d = fromWorkflow(w);
          setSaved(d);
          setDraft(d);
          then?.(w);
        },
      },
    );
  };
  const setTab = (t: 'editor' | 'runs', run?: number) => {
    const next = new URLSearchParams();
    if (t === 'runs') {
      if (run) next.set('run', String(run));
      else next.set('tab', 'runs');
    }
    setParams(next);
  };
  const startRun = (values: ActionValues) =>
    persist(() =>
      run.mutate(values, {
        onSuccess: (r) => {
          setRunOpen(false);
          setTab('runs', r.id);
        },
      }),
    );
  const onRunClick = () => (draft.spec.inputs.length ? setRunOpen(true) : startRun({}));

  const setSteps = (steps: WorkflowStep[]) => setDraft((d) => ({ ...d, spec: { ...d.spec, steps } }));
  const add = (kind: WorkflowStepKind) => {
    if (!adding) return;
    const id = freeId(draft.spec.steps, ID_BASE[kind]);
    setSteps(insertStep(draft.spec.steps, adding.ref, adding.index, starterStep(id, kind)));
    setAdding(null);
    setPanel({ kind: 'node', id });
  };
  const deleteStep = (id: string) => {
    setSteps(removeStep(draft.spec.steps, id));
    if (panel?.kind === 'node' && panel.id === id) setPanel(null);
  };
  const selectedId = panel?.kind === 'node' ? panel.id : null;
  const selectedStep = selectedId && selectedId !== TRIGGER_ID ? allSteps(draft.spec.steps).find((s) => s.id === selectedId) : undefined;

  return (
    <div className="wfe-page">
      <header className="wfe-titlebar">
        <button className="wf-icon-btn" aria-label="Back to workflows" onClick={() => navigate('/automation')}>
          <ArrowLeft size={18} />
        </button>
        <span className="wfe-crumb">
          <Link to="/automation">Workflow Automation</Link> /
        </span>
        <input
          className="wfe-name"
          style={{ width: `${Math.min(48, Math.max(12, draft.name.length + 2))}ch` }}
          aria-label="Workflow name"
          value={draft.name}
          onChange={(e) => setDraft({ ...draft, name: e.target.value })}
        />
        <span className={`wf-chip ${workflow.published ? 'wf-chip-published' : 'wf-chip-draft'}`}>{workflow.published ? 'Published' : 'Draft'}</span>
        {(dirty || workflow.unpublishedChanges) && workflow.published && <span className="wfe-hint">Unpublished changes</span>}
        {dirty && !workflow.published && <span className="wfe-hint">Unsaved changes</span>}
        <div className="wfe-tabs" role="tablist">
          <button role="tab" aria-selected={tab === 'editor'} className={tab === 'editor' ? 'active' : ''} onClick={() => setTab('editor')}>
            Editor
          </button>
          <button role="tab" aria-selected={tab === 'runs'} className={tab === 'runs' ? 'active' : ''} onClick={() => setTab('runs')}>
            Run history
          </button>
        </div>
        <span className="wfi-spacer" />
        {problem && (
          <span className="wfe-problem" title={problem}>
            <AlertTriangle size={14} /> {problem}
          </span>
        )}
        <button className="btn" disabled={busy || !!problem || draft.spec.steps.length === 0} onClick={onRunClick} title={draft.spec.steps.length ? 'Run the saved draft now' : 'Add a step first'}>
          <Play size={13} /> Run
        </button>
        <button className="btn" disabled={busy || !dirty || !!problem} onClick={() => persist()}>
          {save.isPending ? 'Saving…' : 'Save'}
        </button>
        {workflow.published && !dirty && !workflow.unpublishedChanges ? (
          <button className="btn" disabled={busy} onClick={() => unpublish.mutate(workflow.id)}>
            Unpublish
          </button>
        ) : (
          <button className="btn btn-primary" disabled={busy || !!problem || draft.spec.steps.length === 0} onClick={() => persist((w) => publish.mutate(w.id))}>
            Publish
          </button>
        )}
        <button
          className={`wf-icon-btn wfe-gear${panel?.kind === 'settings' ? ' active' : ''}`}
          aria-label="Workflow settings"
          onClick={() => setPanel(panel?.kind === 'settings' ? null : { kind: 'settings' })}
        >
          <Settings size={17} />
        </button>
        <button
          className="wf-icon-btn"
          aria-label="Delete the workflow"
          onClick={() => {
            if (window.confirm(`Delete ${workflow.name} and its runs?`)) del.mutate(workflow.id, { onSuccess: () => navigate('/automation') });
          }}
        >
          <Trash2 size={16} />
        </button>
      </header>
      {error && <div className="wf-error wfe-error">{error.message}</div>}
      {tab === 'runs' ? (
        <RunHistory workflow={workflow} runId={runId} onSelectRun={(id) => setTab('runs', id)} />
      ) : (
        <div className="wfe-body">
          {adding && <CatalogDrawer onPick={add} onClose={() => setAdding(null)} />}
          <Canvas
            steps={draft.spec.steps}
            triggers={draft.spec.triggers}
            handle={draft.handle}
            selected={selectedId}
            onSelect={(id) => {
              setPanel(id ? { kind: 'node', id } : null);
              if (id) setAdding(null);
            }}
            onAdd={(ref, index) => {
              setAdding({ ref, index });
              setPanel(null);
            }}
            onDelete={deleteStep}
            adding={adding}
          />
          {panel?.kind === 'settings' && <SettingsPanel workflow={workflow} draft={draft} onChange={setDraft} onClose={() => setPanel(null)} />}
          {selectedId === TRIGGER_ID && <TriggerPanel spec={draft.spec} handle={draft.handle} onChange={(spec) => setDraft({ ...draft, spec })} onClose={() => setPanel(null)} />}
          {selectedStep && (
            <StepPanel
              spec={draft.spec}
              step={selectedStep}
              takenIds={allSteps(draft.spec.steps)
                .map((s) => s.id)
                .filter((id) => id !== selectedStep.id)}
              onChange={(next) => {
                setSteps(replaceStep(draft.spec.steps, selectedStep.id, next));
                if (next.id !== selectedStep.id) setPanel({ kind: 'node', id: next.id });
              }}
              onDelete={() => deleteStep(selectedStep.id)}
              onClose={() => setPanel(null)}
            />
          )}
        </div>
      )}
      {runOpen && <RunDialog workflow={workflow} draft={draft} onClose={() => setRunOpen(false)} onRun={startRun} pending={busy} error={(run.error as Error) ?? null} />}
    </div>
  );
}

/** /automation/workflows/:id: the canvas editor and the run history. */
export function WorkflowEditorPage() {
  const { id } = useParams();
  const { data, error, isLoading } = useWorkflow(id);
  if (error) {
    return (
      <div className="wfe-page">
        <div className="wfl-empty">
          {(error as Error).message} <Link to="/automation">Back to workflows</Link>
        </div>
      </div>
    );
  }
  if (isLoading || !data) return <div className="wfe-page wfl-empty">Loading the workflow…</div>;
  return <Editor key={data.id} workflow={data} />;
}
