import { useMemo, useState } from 'react';
import { ChevronDown, ChevronRight, CircleStop, History } from 'lucide-react';
import { STEP_LABEL, TRIGGER_LABEL } from '../../../api/workflows/logic';
import type { JsonValue } from '../../../api/types';
import type { Workflow, WorkflowRun, WorkflowRunStep } from '../../../api/workflows/types';
import { JsonTree } from '../../../components/json/JsonTree';
import { useCancelWorkflowRun, useWorkflowRun, useWorkflowRuns } from '../queries';
import { Canvas } from './Canvas';
import { fmtDate, fmtDuration, RunStatusChip, StepTile, TRIGGER_ICON } from './common';
import { fromWorkflow } from './draft';

function Json({ value }: { value: JsonValue }) {
  if (value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length) return <JsonTree data={value as Record<string, JsonValue>} expandDepth={2} />;
  if (value === null || (typeof value === 'object' && !Array.isArray(value))) return <span className="wf-muted">—</span>;
  return <pre className="wfr-pre mono">{JSON.stringify(value, null, 2)}</pre>;
}

function StepRow({ s, selected, onSelect }: { s: WorkflowRunStep; selected: boolean; onSelect: () => void }) {
  const [open, setOpen] = useState(selected || s.status === 'failed');
  return (
    <div className={`wfr-step${selected ? ' selected' : ''}`}>
      <button
        className="wfr-step-head"
        onClick={() => {
          setOpen((o) => !o);
          onSelect();
        }}
      >
        {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        <StepTile kind={s.kind} size={24} />
        <span className="wfr-step-name">
          {s.name || STEP_LABEL[s.kind]} <span className="wf-muted mono">{s.stepId}</span>
          {s.iteration !== null && s.iteration !== undefined && <span className="wfr-iter">item {s.iteration}</span>}
        </span>
        <RunStatusChip status={s.status} />
        <span className="wf-muted wfr-dur">{fmtDuration(s.durationMs)}</span>
      </button>
      {open && (
        <div className="wfr-step-body">
          {s.error && <div className="wf-error">{s.error}</div>}
          {s.attempts > 1 && <div className="wf-muted">{s.attempts} attempts</div>}
          <div className="wfr-io">
            <div>
              <div className="wfv-group">Inputs</div>
              <Json value={s.inputs} />
            </div>
            <div>
              <div className="wfv-group">Output</div>
              <Json value={s.output} />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function RunDetail({ workflow, runId }: { workflow: Workflow; runId: number }) {
  const { data: run, error } = useWorkflowRun(workflow.id, runId);
  const cancel = useCancelWorkflowRun(workflow.id);
  const [focus, setFocus] = useState<string | null>(null);
  const spec = run?.spec ?? workflow.publishedSpec ?? workflow.spec;
  const shown = useMemo(() => fromWorkflow({ ...workflow, spec }).spec, [workflow, spec]);
  const statuses = useMemo(() => {
    const out: Record<string, 'succeeded' | 'failed'> = {};
    for (const s of run?.steps ?? []) out[s.stepId] = out[s.stepId] === 'failed' ? 'failed' : s.status;
    return out;
  }, [run]);
  if (error) return <div className="wf-error">{(error as Error).message}</div>;
  if (!run) return <div className="wfl-empty">Loading the run…</div>;
  const active = run.status === 'queued' || run.status === 'running' || run.status === 'waiting';
  const duration = run.startedAt ? (run.finishedAt ?? Date.now()) - run.startedAt : null;
  return (
    <div className="wfr-detail">
      <div className="wfr-detail-head">
        <h3>Run #{run.id}</h3>
        <RunStatusChip status={run.status} />
        <span className="wf-muted">
          {TRIGGER_LABEL[run.triggerKind]} · {run.triggeredBy} · {fmtDate(run.createdAt)} · {fmtDuration(duration)}
        </span>
        <span className="wfi-spacer" />
        {active && (
          <button className="btn btn-sm" disabled={cancel.isPending} onClick={() => cancel.mutate(run.id)}>
            <CircleStop size={13} /> Cancel run
          </button>
        )}
      </div>
      {run.error && <div className="wf-error">{run.error}</div>}
      {run.status === 'waiting' && run.nextAt && <div className="wf-note">Waiting until {fmtDate(run.nextAt)}.</div>}
      <div className="wfr-split">
        <div className="wfr-canvas">
          <Canvas steps={shown.steps} triggers={shown.triggers} handle={workflow.handle} selected={focus} onSelect={setFocus} statuses={statuses} />
        </div>
        <div className="wfr-steps">
          {Object.keys(run.inputs ?? {}).length > 0 && (
            <div className="wfr-block">
              <div className="wfv-group">Run inputs</div>
              <Json value={run.inputs as JsonValue} />
            </div>
          )}
          <div className="wfr-block">
            <div className="wfv-group">Trigger</div>
            <Json value={run.trigger as JsonValue} />
          </div>
          {run.steps.map((s) => (
            <StepRow key={s.id} s={s} selected={focus === s.stepId} onSelect={() => setFocus(s.stepId)} />
          ))}
          {run.steps.length === 0 && <div className="wf-muted wfr-block">{active ? 'Starting…' : 'No step ran.'}</div>}
          {active && <div className="wf-muted wfr-block">Running… this view refreshes by itself.</div>}
        </div>
      </div>
    </div>
  );
}

/** The editor's "Run history" tab: the runs, and the selected one step by step. */
export function RunHistory({ workflow, runId, onSelectRun }: { workflow: Workflow; runId: number | null; onSelectRun: (id: number) => void }) {
  const { data: runs, isLoading } = useWorkflowRuns(workflow.id);
  const selected = runId ?? runs?.[0]?.id ?? null;
  return (
    <div className="wfr-page">
      <aside className="wfr-list">
        <div className="wfr-list-head">
          <History size={14} /> Runs <span className="wf-muted">{runs?.length ?? 0}</span>
        </div>
        {isLoading && <div className="wf-muted wfr-block">Loading…</div>}
        {runs?.length === 0 && <div className="wf-muted wfr-block">No runs yet. Use ▶ Run, or publish it so its triggers start runs.</div>}
        {runs?.map((r: WorkflowRun) => {
          const duration = r.startedAt && r.finishedAt ? r.finishedAt - r.startedAt : null;
          return (
            <button key={r.id} className={`wfr-run${r.id === selected ? ' selected' : ''}`} onClick={() => onSelectRun(r.id)}>
              <div className="wfr-run-top">
                <RunStatusChip status={r.status} />
                <span className="wfr-run-id">#{r.id}</span>
                <span className="wfi-spacer" />
                <span className="wf-muted">{fmtDuration(duration)}</span>
              </div>
              <div className="wfr-run-sub wf-muted">
                {TRIGGER_ICON[r.triggerKind]} {r.triggeredBy} · {fmtDate(r.createdAt)}
              </div>
            </button>
          );
        })}
      </aside>
      <section className="wfr-main">{selected ? <RunDetail key={selected} workflow={workflow} runId={selected} /> : <div className="wfl-empty">Select a run</div>}</section>
    </div>
  );
}
