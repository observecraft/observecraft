import { useState } from 'react';
import { AlertTriangle, X } from 'lucide-react';
import type { AppComponent, AppSpec } from '../../../api/apps/types';
import type { Workflow } from '../../../api/workflows/types';
import { AGGREGATIONS, componentProblem, KIND_LABEL, pathSuggestions, WIDTHS } from './appLogic';

interface Props {
  c: AppComponent;
  spec: AppSpec;
  workflows: Workflow[];
  onChange: (next: AppComponent) => void;
  onRename: (to: string) => void;
}

const canRunFromApp = (w: Workflow) => !!w.publishedSpec && w.publishedSpec.triggers.some((t) => t.kind === 'app');

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="ab-prop">
      <span className="ab-prop-label">{label}</span>
      {children}
      {hint && <span className="ab-prop-hint">{hint}</span>}
    </div>
  );
}

/** The properties of the selected component. */
export function BuilderProperties({ c, spec, workflows, onChange, onRename }: Props) {
  const [id, setId] = useState(c.id);
  const set = (patch: Partial<AppComponent>) => onChange({ ...c, ...patch });
  const problem = componentProblem(c, spec);
  const idTaken = id !== c.id && spec.components.some((x) => x.id === id);
  const idValid = /^[a-z0-9_]{1,40}$/.test(id);
  return (
    <div className="ab-props" key={c.id}>
      <div className="ab-props-head">
        <span className="ab-kind-pill">{KIND_LABEL[c.kind]}</span>
      </div>
      {problem && (
        <div className="ab-problem">
          <AlertTriangle size={13} /> {problem}
        </div>
      )}
      <Field label="Component ID" hint={idTaken ? 'Another component has this ID' : !idValid ? 'a–z, 0–9 and _, up to 40' : `Read in templates as {{ components.${c.id}.value }}`}>
        <input className="ab-input mono" value={id} onChange={(e) => setId(e.target.value)} onBlur={() => (idValid && !idTaken && id !== c.id ? onRename(id) : setId(c.id))} />
      </Field>
      {c.kind !== 'heading' && c.kind !== 'text' && (
        <Field label="Label">
          <input className="ab-input" value={c.label} onChange={(e) => set({ label: e.target.value })} />
        </Field>
      )}
      <Field label="Width">
        <div className="ab-seg">
          {WIDTHS.map((w) => (
            <button key={w} className={c.width === w ? 'active' : ''} onClick={() => set({ width: w })}>
              {w === 12 ? 'Full' : `${w}/12`}
            </button>
          ))}
        </div>
      </Field>
      {(c.kind === 'heading' || c.kind === 'text') && (
        <Field label="Text">
          {c.kind === 'heading' ? (
            <input className="ab-input" value={c.text} onChange={(e) => set({ text: e.target.value })} />
          ) : (
            <textarea className="ab-input ab-textarea" value={c.text} onChange={(e) => set({ text: e.target.value })} />
          )}
        </Field>
      )}
      {(c.kind === 'text_input' || c.kind === 'number_input' || c.kind === 'select') && (
        <>
          {c.kind === 'select' ? (
            <Field label="Options" hint="One per line">
              <textarea
                className="ab-input ab-textarea mono"
                value={c.options.join('\n')}
                onChange={(e) => set({ options: e.target.value.split('\n') })}
                onBlur={() => set({ options: c.options.map((o) => o.trim()).filter(Boolean) })}
              />
            </Field>
          ) : (
            <Field label="Placeholder">
              <input className="ab-input" value={c.placeholder} onChange={(e) => set({ placeholder: e.target.value })} />
            </Field>
          )}
          <Field label="Default value">
            {c.kind === 'select' ? (
              <select className="ab-input" value={c.defaultValue} onChange={(e) => set({ defaultValue: e.target.value })}>
                <option value="">(first option)</option>
                {c.options.filter(Boolean).map((o) => (
                  <option key={o}>{o}</option>
                ))}
              </select>
            ) : (
              <input className="ab-input" type={c.kind === 'number_input' ? 'number' : 'text'} value={c.defaultValue} onChange={(e) => set({ defaultValue: e.target.value })} />
            )}
          </Field>
        </>
      )}
      {c.kind === 'button' && <ButtonProps c={c} spec={spec} workflows={workflows} set={set} />}
      {c.kind === 'table' && <TableProps c={c} spec={spec} workflows={workflows} set={set} />}
      {c.kind === 'metric' && (
        <>
          <Field label="Metric query" hint="A metric query or formula, evaluated over the past 15 minutes">
            <textarea className="ab-input ab-textarea mono" value={c.query} onChange={(e) => set({ query: e.target.value })} />
          </Field>
          <Field label="Aggregation">
            <select className="ab-input" value={c.aggregation || 'avg'} onChange={(e) => set({ aggregation: e.target.value })}>
              {AGGREGATIONS.map((a) => (
                <option key={a}>{a}</option>
              ))}
            </select>
          </Field>
          <Field label="Unit">
            <input className="ab-input" value={c.unit} placeholder="req/s" onChange={(e) => set({ unit: e.target.value })} />
          </Field>
        </>
      )}
    </div>
  );
}

function ButtonProps({ c, spec, workflows, set }: { c: AppComponent; spec: AppSpec; workflows: Workflow[]; set: (p: Partial<AppComponent>) => void }) {
  const wf = workflows.find((w) => w.id === c.workflowId);
  const keys = (wf?.publishedSpec?.inputs ?? wf?.spec.inputs ?? []).map((i) => i.key);
  const extra = Object.keys(c.inputs).filter((k) => !keys.includes(k));
  const inputs = spec.components.filter((x) => x.kind === 'text_input' || x.kind === 'number_input' || x.kind === 'select');
  const setInput = (key: string, value: string) => {
    const next = { ...c.inputs };
    if (value) next[key] = value;
    else delete next[key];
    set({ inputs: next });
  };
  const runnable = workflows.filter(canRunFromApp);
  const others = workflows.filter((w) => !canRunFromApp(w));
  return (
    <>
      <Field label="Workflow" hint={wf && !canRunFromApp(wf) ? 'Publish this workflow with an App trigger so the button can run it' : 'Runs as the member who published the workflow'}>
        <select className="ab-input" value={c.workflowId} onChange={(e) => set({ workflowId: e.target.value })}>
          <option value="">Choose a workflow…</option>
          <optgroup label="Published, with an App trigger">
            {runnable.map((w) => (
              <option key={w.id} value={w.id}>
                {w.name}
              </option>
            ))}
          </optgroup>
          {others.length > 0 && (
            <optgroup label="Not runnable from apps yet">
              {others.map((w) => (
                <option key={w.id} value={w.id}>
                  {w.name}
                </option>
              ))}
            </optgroup>
          )}
        </select>
      </Field>
      {wf && (
        <div className="ab-prop">
          <span className="ab-prop-label">Workflow inputs</span>
          {keys.length === 0 && extra.length === 0 && <span className="ab-prop-hint">This workflow has no inputs.</span>}
          <table className="ab-map">
            <tbody>
              {[...keys, ...extra].map((k) => (
                <tr key={k}>
                  <td className="mono">
                    {k}
                    {extra.includes(k) && <span className="ab-prop-hint"> (not an input)</span>}
                  </td>
                  <td>
                    <input className="ab-input mono" value={c.inputs[k] ?? ''} placeholder="{{ components.<id>.value }}" onChange={(e) => setInput(k, e.target.value)} />
                  </td>
                  <td>
                    <select className="ab-pick" aria-label={`Insert a component into ${k}`} value="" onChange={(e) => e.target.value && setInput(k, `{{ components.${e.target.value}.value }}`)}>
                      <option value="">Insert…</option>
                      {inputs.map((x) => (
                        <option key={x.id} value={x.id}>
                          {x.label || x.id}
                        </option>
                      ))}
                    </select>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function TableProps({ c, spec, workflows, set }: { c: AppComponent; spec: AppSpec; workflows: Workflow[]; set: (p: Partial<AppComponent>) => void }) {
  const buttons = spec.components.filter((x) => x.kind === 'button');
  const source = buttons.find((b) => b.id === c.source);
  const wf = workflows.find((w) => w.id === source?.workflowId);
  const suggestions = pathSuggestions(wf?.publishedSpec ?? wf?.spec);
  const [column, setColumn] = useState('');
  return (
    <>
      <Field label="Data from" hint="The table shows the latest run of this button">
        <select className="ab-input" value={c.source} onChange={(e) => set({ source: e.target.value })}>
          <option value="">Choose a button…</option>
          {buttons.map((b) => (
            <option key={b.id} value={b.id}>
              {b.label || b.id}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Path" hint="Where the list is in the run: Steps.<step>.output.<field>">
        <input className="ab-input mono" list={`ab-paths-${c.id}`} value={c.path} onChange={(e) => set({ path: e.target.value })} />
        <datalist id={`ab-paths-${c.id}`}>
          {suggestions.map((s) => (
            <option key={s} value={s} />
          ))}
        </datalist>
      </Field>
      {suggestions.length > 0 && (
        <div className="ab-suggest">
          {suggestions.slice(0, 6).map((s) => (
            <button key={s} className="ab-suggest-chip mono" onClick={() => set({ path: s })}>
              {s}
            </button>
          ))}
        </div>
      )}
      <div className="ab-prop">
        <span className="ab-prop-label">Columns</span>
        <div className="ab-chips">
          {c.columns.map((col) => (
            <span key={col} className="ab-col-chip mono">
              {col}
              <button aria-label={`Remove ${col}`} onClick={() => set({ columns: c.columns.filter((x) => x !== col) })}>
                <X size={11} />
              </button>
            </span>
          ))}
        </div>
        <div className="ab-inline">
          <input
            className="ab-input mono"
            placeholder="field name"
            value={column}
            onChange={(e) => setColumn(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && column.trim() && (set({ columns: [...c.columns, column.trim()] }), setColumn(''))}
          />
          <button className="btn" disabled={!column.trim()} onClick={() => (set({ columns: [...c.columns, column.trim()] }), setColumn(''))}>
            Add
          </button>
        </div>
        <span className="ab-prop-hint">Every field of the rows when empty</span>
      </div>
    </>
  );
}
