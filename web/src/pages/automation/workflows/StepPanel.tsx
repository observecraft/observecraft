import { Plus, Trash2, X } from 'lucide-react';
import { exprError, STEP_FIELDS, STEP_LABEL, templateError } from '../../../api/workflows/logic';
import type { ActionHttpRequest } from '../../../api/portalActions/types';
import type { WorkflowSpec, WorkflowStep } from '../../../api/workflows/types';
import { StepTile } from './common';
import { TemplateField } from './TemplateField';
import { variablesFor } from './variables';

const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'];

/** Labels, hints and placeholders of the fields. */
const FIELD_INFO: Record<string, { label: string; hint?: string; placeholder?: string; multiline?: boolean; mono?: boolean }> = {
  title: { label: 'Title' },
  severity: { label: 'Severity', hint: 'SEV-1 … SEV-5 or UNKNOWN; SEV-3 when empty', placeholder: 'SEV-3' },
  summary: { label: 'Summary', multiline: true },
  teams: { label: 'Teams', hint: 'Comma-separated team handles', placeholder: 'checkout, payments' },
  services: { label: 'Services', hint: 'Comma-separated', placeholder: 'order-service' },
  team: { label: 'Team', hint: 'The on-call team handle', placeholder: 'payments' },
  description: { label: 'Description', multiline: true },
  urgency: { label: 'Urgency', hint: 'high or low', placeholder: 'high' },
  incident: { label: 'Incident', hint: 'Its number: 12, #12 or IR-12', placeholder: '{{ Steps.declare.output.incident }}' },
  body: { label: 'Note', multiline: true },
  test: { label: 'Synthetic test id' },
  recipients: { label: 'Recipients', hint: '@slack-…, @webhook-… channels or member emails, comma-separated', placeholder: '@slack-checkout' },
  message: { label: 'Message', multiline: true },
  query: { label: 'Query', mono: true },
  minutes: { label: 'Look back (minutes)', hint: '1–1440; 15 when empty', placeholder: '15' },
  aggregation: { label: 'Aggregation', hint: 'avg, sum, min, max or last', placeholder: 'avg' },
  group_by: { label: 'Group by', hint: 'A facet, e.g. service', placeholder: 'service' },
  seconds: { label: 'Seconds', hint: '1–3600; the run resumes afterwards', placeholder: '60' },
};

interface Props {
  spec: WorkflowSpec;
  step: WorkflowStep;
  onChange: (next: WorkflowStep) => void;
  onDelete: () => void;
  onClose: () => void;
  /** Ids used by other steps. */
  takenIds: string[];
}

function RequestEditor({
  r,
  onChange,
  onRemove,
  variables,
  index,
  count,
}: {
  r: ActionHttpRequest;
  onChange: (r: ActionHttpRequest) => void;
  onRemove: () => void;
  variables: ReturnType<typeof variablesFor>;
  index: number;
  count: number;
}) {
  return (
    <div className="wfp-request">
      <div className="wfp-request-head">
        <span>Request {index + 1}</span>
        {count > 1 && (
          <button className="wf-icon-btn" aria-label="Remove request" onClick={onRemove}>
            <Trash2 size={13} />
          </button>
        )}
      </div>
      <div className="wfp-row">
        <select className="wf-input wfp-method" value={r.method} onChange={(e) => onChange({ ...r, method: e.target.value })}>
          {METHODS.map((m) => (
            <option key={m}>{m}</option>
          ))}
        </select>
        <div className="wfp-grow">
          <TemplateField label="URL" mono value={r.url} onChange={(url) => onChange({ ...r, url })} variables={variables} hint="Scheme and host are literal; values are percent-encoded" required />
        </div>
      </div>
      <div className="wf-field-label">Headers</div>
      {(r.headers ?? []).map((h, i) => (
        <div key={i} className="wfp-row">
          <input
            className="wf-input wfp-header-name"
            placeholder="Name"
            value={h.name}
            onChange={(e) => onChange({ ...r, headers: (r.headers ?? []).map((x, j) => (j === i ? { ...x, name: e.target.value } : x)) })}
          />
          <input
            className="wf-input mono wfp-grow"
            placeholder="Value, e.g. Bearer {{secrets.token}}"
            value={h.value}
            onChange={(e) => onChange({ ...r, headers: (r.headers ?? []).map((x, j) => (j === i ? { ...x, value: e.target.value } : x)) })}
          />
          <button className="wf-icon-btn" aria-label="Remove header" onClick={() => onChange({ ...r, headers: (r.headers ?? []).filter((_, j) => j !== i) })}>
            <X size={13} />
          </button>
        </div>
      ))}
      <button className="wf-link-btn" onClick={() => onChange({ ...r, headers: [...(r.headers ?? []), { name: '', value: '' }] })}>
        <Plus size={12} /> Add header
      </button>
      <TemplateField
        label="JSON body"
        mono
        multiline
        value={r.body ?? ''}
        onChange={(body) => onChange({ ...r, body })}
        variables={variables}
        placeholder='{"reason": "{{ Trigger.monitor.name }}"}'
        hint="Placeholders go inside strings"
      />
    </div>
  );
}

/** Right-side panel configuring the selected step. */
export function StepPanel({ spec, step, onChange, onDelete, onClose, takenIds }: Props) {
  const variables = variablesFor(spec, step.id);
  const keys = spec.inputs.map((i) => i.key);
  const idError = !/^[a-z0-9_]{1,40}$/.test(step.id) ? 'a–z, 0–9 and _, up to 40' : takenIds.includes(step.id) ? 'Another step uses this id' : null;
  const set = (patch: Partial<WorkflowStep>) => onChange({ ...step, ...patch });
  const control = step.kind === 'if' || step.kind === 'for_each';
  return (
    <aside className="wfe-panel" onClick={(e) => e.stopPropagation()}>
      <div className="wfe-panel-head">
        <StepTile kind={step.kind} />
        <div className="wfe-panel-title">
          <div>{step.name || STEP_LABEL[step.kind]}</div>
          <div className="wf-muted">{STEP_LABEL[step.kind]}</div>
        </div>
        <button className="wf-icon-btn" aria-label="Close" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <div className="wfe-panel-body">
        <label className="wf-field">
          <span className="wf-field-label">Step id</span>
          <input className={`wf-input mono${idError ? ' wf-input-error' : ''}`} value={step.id} onChange={(e) => set({ id: e.target.value.toLowerCase() })} />
          {idError ? <span className="wf-field-error">{idError}</span> : <span className="wf-field-hint mono">{`{{ Steps.${step.id}.output }}`}</span>}
        </label>
        <label className="wf-field">
          <span className="wf-field-label">Name</span>
          <input className="wf-input" value={step.name} placeholder={STEP_LABEL[step.kind]} onChange={(e) => set({ name: e.target.value })} />
        </label>
        {control && (
          <TemplateField
            label={step.kind === 'if' ? 'Condition' : 'List'}
            mono
            bare
            required
            value={step.expression}
            onChange={(expression) => set({ expression })}
            variables={variables}
            error={exprError(step.expression)}
            hint={step.kind === 'if' ? "e.g. Steps.check.output.value > 5 && Trigger.transition.to == 'alert'" : 'A list, e.g. Steps.top_errors.output.groups (up to 100 items); read Loop.item inside'}
          />
        )}
        {step.kind === 'http' && (
          <>
            {step.requests.map((r, i) => (
              <RequestEditor
                key={i}
                r={r}
                index={i}
                count={step.requests.length}
                variables={variables}
                onChange={(nr) => set({ requests: step.requests.map((x, j) => (j === i ? nr : x)) })}
                onRemove={() => set({ requests: step.requests.filter((_, j) => j !== i) })}
              />
            ))}
            {step.requests.length < 5 && (
              <button className="wf-link-btn" onClick={() => set({ requests: [...step.requests, { method: 'GET', url: 'https://', headers: [], body: '' }] })}>
                <Plus size={12} /> Add a request (later ones read earlier responses as {'{{steps.1.<field>}}'})
              </button>
            )}
          </>
        )}
        {STEP_FIELDS[step.kind].map(([f, required]) => {
          const info = FIELD_INFO[f] ?? { label: f };
          const value = step.fields[f] ?? '';
          return (
            <TemplateField
              key={f}
              label={info.label}
              required={required}
              multiline={info.multiline}
              mono={info.mono}
              value={value}
              placeholder={info.placeholder}
              hint={info.hint}
              error={(required && !value.trim() ? 'Required' : null) ?? templateError(value, keys)}
              variables={variables}
              onChange={(v) => set({ fields: { ...step.fields, [f]: v } })}
            />
          );
        })}
        {!control && (
          <div className="wfp-options">
            <label className="wf-field wfp-retries">
              <span className="wf-field-label">Retries</span>
              <select className="wf-input" value={step.retries} onChange={(e) => set({ retries: Number(e.target.value) })}>
                {[0, 1, 2, 3].map((n) => (
                  <option key={n} value={n}>
                    {n === 0 ? 'No retry' : `${n} (5 s apart)`}
                  </option>
                ))}
              </select>
            </label>
            <label className="wfp-check">
              <input type="checkbox" className="checkbox" checked={step.continueOnError} onChange={(e) => set({ continueOnError: e.target.checked })} />
              Continue the run when this step fails
            </label>
          </div>
        )}
      </div>
      <div className="wfe-panel-foot">
        <button className="btn btn-sm btn-danger" onClick={onDelete}>
          <Trash2 size={13} /> Delete step
        </button>
      </div>
    </aside>
  );
}
