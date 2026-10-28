import { Plus, Trash2, X, Zap } from 'lucide-react';
import { describeCron, emptyTrigger, TRIGGER_LABEL } from '../../../api/workflows/logic';
import type { ActionInput, ActionInputKind } from '../../../api/portalActions/types';
import type { WorkflowSpec, WorkflowTrigger, WorkflowTriggerKind } from '../../../api/workflows/types';
import { TRIGGER_ICON } from './common';

const KINDS: WorkflowTriggerKind[] = ['manual', 'monitor', 'schedule', 'incident', 'api', 'app'];
const TRANSITIONS = ['alert', 'warn', 'nodata', 'ok'];
const EVENTS = ['declared', 'updated', 'resolved'];
const SEVERITIES = ['SEV-1', 'SEV-2', 'SEV-3', 'SEV-4', 'SEV-5', 'UNKNOWN'];
const INPUT_KINDS: ActionInputKind[] = ['text', 'select', 'number', 'boolean'];
const ZONES = ['UTC', 'Europe/London', 'Europe/Paris', 'Europe/Berlin', 'America/New_York', 'America/Chicago', 'America/Los_Angeles', 'Asia/Tokyo', 'Asia/Singapore', 'Australia/Sydney'];

const HELP: Record<WorkflowTriggerKind, string> = {
  manual: 'Run it from the editor or the workflow list; the run acts as you and runs the saved draft.',
  monitor: 'Mention the handle in a monitor’s message; the published workflow runs on the states below.',
  schedule: 'Runs the published workflow at these times.',
  incident: 'Runs the published workflow when an incident matches.',
  api: 'POST /api/v1/workflows/{id}/trigger with an application key and the workflow’s token (see Settings).',
  app: 'Buttons of apps run the published workflow with the inputs below.',
};

function Toggles({ options, value, onChange, empty }: { options: string[]; value: string[]; onChange: (v: string[]) => void; empty: string }) {
  return (
    <div className="wft-toggles">
      {options.map((o) => (
        <button key={o} className={`wft-toggle${value.includes(o) ? ' on' : ''}`} onClick={() => onChange(value.includes(o) ? value.filter((x) => x !== o) : [...value, o])}>
          {o}
        </button>
      ))}
      {value.length === 0 && <span className="wf-muted">{empty}</span>}
    </div>
  );
}

function TriggerCard({ t, handle, onChange, onRemove }: { t: WorkflowTrigger; handle: string; onChange: (t: WorkflowTrigger) => void; onRemove: () => void }) {
  return (
    <div className="wft-card">
      <div className="wft-card-head">
        <span className="wft-kind">
          {TRIGGER_ICON[t.kind]} {TRIGGER_LABEL[t.kind]}
        </span>
        <button className="wf-icon-btn" aria-label={`Remove ${t.kind} trigger`} onClick={onRemove}>
          <Trash2 size={13} />
        </button>
      </div>
      <p className="wf-muted wft-help">{HELP[t.kind]}</p>
      {t.kind === 'monitor' && (
        <>
          <div className="wft-handle mono">@workflow-{handle || '…'}</div>
          <div className="wf-field-label">Run on</div>
          <Toggles options={TRANSITIONS} value={t.transitions} onChange={(transitions) => onChange({ ...t, transitions })} empty="alert only" />
        </>
      )}
      {t.kind === 'schedule' && (
        <>
          <label className="wf-field">
            <span className="wf-field-label">Cron (minute hour day month weekday)</span>
            <input className="wf-input mono" value={t.cron} onChange={(e) => onChange({ ...t, cron: e.target.value })} placeholder="0 9 * * 1-5" />
          </label>
          <label className="wf-field">
            <span className="wf-field-label">Time zone</span>
            <select className="wf-input" value={t.timezone || 'UTC'} onChange={(e) => onChange({ ...t, timezone: e.target.value })}>
              {[...new Set([t.timezone || 'UTC', ...ZONES])].map((z) => (
                <option key={z}>{z}</option>
              ))}
            </select>
          </label>
          <div className="wft-cron-desc">{describeCron(t.cron, t.timezone)}</div>
        </>
      )}
      {t.kind === 'incident' && (
        <>
          <div className="wf-field-label">Events</div>
          <Toggles options={EVENTS} value={t.incidentEvents} onChange={(incidentEvents) => onChange({ ...t, incidentEvents })} empty="all events" />
          <div className="wf-field-label">Severities</div>
          <Toggles options={SEVERITIES} value={t.severities} onChange={(severities) => onChange({ ...t, severities })} empty="any severity" />
        </>
      )}
    </div>
  );
}

function InputsEditor({ inputs, onChange }: { inputs: ActionInput[]; onChange: (i: ActionInput[]) => void }) {
  const set = (i: number, patch: Partial<ActionInput>) => onChange(inputs.map((x, j) => (j === i ? { ...x, ...patch } : x)));
  return (
    <div className="wft-inputs">
      {inputs.map((inp, i) => (
        <div key={i} className="wft-input-row">
          <input className="wf-input mono" placeholder="key" value={inp.key} onChange={(e) => set(i, { key: e.target.value.toLowerCase() })} />
          <input className="wf-input" placeholder="Label" value={inp.label} onChange={(e) => set(i, { label: e.target.value })} />
          <select className="wf-input" value={inp.kind} onChange={(e) => set(i, { kind: e.target.value as ActionInputKind })}>
            {INPUT_KINDS.map((k) => (
              <option key={k}>{k}</option>
            ))}
          </select>
          <button className="wf-icon-btn" aria-label="Remove input" onClick={() => onChange(inputs.filter((_, j) => j !== i))}>
            <X size={13} />
          </button>
          {inp.kind === 'select' && (
            <input
              className="wf-input wft-wide"
              placeholder="Options, comma-separated"
              value={(inp.options ?? []).join(', ')}
              onChange={(e) =>
                set(i, {
                  options: e.target.value
                    .split(',')
                    .map((o) => o.trim())
                    .filter(Boolean),
                })
              }
            />
          )}
          <input className="wf-input wft-wide" placeholder="Default value" value={inp.default ?? ''} onChange={(e) => set(i, { default: e.target.value })} />
          <label className="wfp-check wft-wide">
            <input type="checkbox" className="checkbox" checked={!!inp.required} onChange={(e) => set(i, { required: e.target.checked })} /> Required
          </label>
        </div>
      ))}
      <button className="wf-link-btn" onClick={() => onChange([...inputs, { key: `input_${inputs.length + 1}`, label: '', kind: 'text', required: false, options: [], default: '', description: '' }])}>
        <Plus size={12} /> Add an input
      </button>
    </div>
  );
}

/** Right-side panel of the trigger node: the triggers and the run's form inputs. */
export function TriggerPanel({ spec, handle, onChange, onClose }: { spec: WorkflowSpec; handle: string; onChange: (s: WorkflowSpec) => void; onClose: () => void }) {
  const missing = KINDS.filter((k) => k === 'schedule' || !spec.triggers.some((t) => t.kind === k));
  return (
    <aside className="wfe-panel" onClick={(e) => e.stopPropagation()}>
      <div className="wfe-panel-head">
        <span className="wf-tile wf-tone-orange">
          <Zap size={16} />
        </span>
        <div className="wfe-panel-title">
          <div>Triggers</div>
          <div className="wf-muted">What starts this workflow</div>
        </div>
        <button className="wf-icon-btn" aria-label="Close" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <div className="wfe-panel-body">
        {spec.triggers.map((t, i) => (
          <TriggerCard
            key={i}
            t={t}
            handle={handle}
            onChange={(nt) => onChange({ ...spec, triggers: spec.triggers.map((x, j) => (j === i ? nt : x)) })}
            onRemove={() => onChange({ ...spec, triggers: spec.triggers.filter((_, j) => j !== i) })}
          />
        ))}
        {spec.triggers.length < 5 && (
          <div className="wft-add">
            <div className="wf-field-label">Add a trigger</div>
            <div className="wfn-chips">
              {missing.map((k) => (
                <button key={k} className="wfn-chip" onClick={() => onChange({ ...spec, triggers: [...spec.triggers, emptyTrigger(k)] })}>
                  {TRIGGER_ICON[k]} {TRIGGER_LABEL[k]}
                </button>
              ))}
            </div>
          </div>
        )}
        <div className="wft-section">Inputs</div>
        <p className="wf-muted wft-help">
          A form for manual, app and API runs; steps read the values as <span className="mono">{'{{ inputs.<key> }}'}</span>.
        </p>
        <InputsEditor inputs={spec.inputs} onChange={(inputs) => onChange({ ...spec, inputs })} />
      </div>
    </aside>
  );
}
