import { Siren } from 'lucide-react';
import type { ActionInput, ActionValues } from '../../../api/portalActions/client';
import { IntegrationLogo } from '../../integrations/IntegrationLogo';

/** The tool an action talks to: its brand logo, or the ObserveCraft mark for internal steps. */
export function ToolLogo({ logo, size = 20 }: { logo: string; size?: number }) {
  if (logo === 'internal') return <Siren size={size} className="psa-internal-logo" aria-label="ObserveCraft" />;
  return <IntegrationLogo logo={logo} name={logo} size={size} />;
}

/** The logo for a saved action: from its blueprint, else from its step. */
export function actionLogo(blueprintLogo: string | undefined, stepKind: string, url = ''): string {
  if (blueprintLogo) return blueprintLogo;
  if (stepKind !== 'http') return 'internal';
  if (url.includes('github')) return 'github';
  if (url.includes('gitlab')) return 'gitlab';
  if (url.includes('atlassian')) return 'jira';
  return 'webhook';
}

export function RunStatus({ status }: { status: string | null | undefined }) {
  if (!status) return <span className="psa-muted">Never run</span>;
  return <span className={`psa-status psa-status-${status === 'succeeded' ? 'ok' : 'error'}`}>{status === 'succeeded' ? 'Succeeded' : 'Failed'}</span>;
}

/** The form's starting values: defaults, booleans false when unset. */
export function initialValues(inputs: ActionInput[]): ActionValues {
  const out: ActionValues = {};
  for (const i of inputs) {
    if (i.kind === 'boolean') out[i.key] = i.default === 'true';
    else out[i.key] = i.default ?? '';
  }
  return out;
}

/** The form generated from an action's inputs. */
export function ActionForm({ inputs, values, onChange, readOnly = false }: { inputs: ActionInput[]; values: ActionValues; onChange?: (v: ActionValues) => void; readOnly?: boolean }) {
  const set = (key: string, v: string | boolean) => onChange?.({ ...values, [key]: v });
  if (!inputs.length) return <p className="psa-muted">This action has no inputs: it runs as it is.</p>;
  return (
    <div className="psa-form">
      {inputs.map((i) => {
        const id = `psa-in-${i.key}`;
        const label = (
          <span className="psa-form-label">
            {i.label}
            {i.required && <span className="psa-required"> *</span>}
          </span>
        );
        if (i.kind === 'boolean') {
          return (
            <label key={i.key} className="psa-form-check">
              <input type="checkbox" checked={values[i.key] === true} disabled={readOnly} onChange={(e) => set(i.key, e.target.checked)} />
              {label}
              {i.description && <small className="psa-muted">{i.description}</small>}
            </label>
          );
        }
        return (
          <div key={i.key} className="sc-field">
            <label htmlFor={id}>{label}</label>
            {i.kind === 'select' ? (
              <select id={id} className="sc-input" value={String(values[i.key] ?? '')} disabled={readOnly} onChange={(e) => set(i.key, e.target.value)}>
                {!i.required && <option value="">—</option>}
                {(i.options ?? []).map((o) => (
                  <option key={o} value={o}>
                    {o}
                  </option>
                ))}
              </select>
            ) : (
              <input id={id} className="sc-input" type={i.kind === 'number' ? 'number' : 'text'} value={String(values[i.key] ?? '')} readOnly={readOnly} onChange={(e) => set(i.key, e.target.value)} />
            )}
            {i.description && <small className="psa-muted">{i.description}</small>}
          </div>
        );
      })}
    </div>
  );
}
