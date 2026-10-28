import { useEffect, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ArrowDown, ArrowLeft, ArrowUp, Plus, Trash2 } from 'lucide-react';
import { blueprint as findBlueprint } from '../../../api/portalActions/blueprints';
import { CATEGORY_LABEL, METHODS, secretsUsed, STEP_FIELDS, STEP_LABEL } from '../../../api/portalActions/logic';
import type { ActionHttpRequest, ActionInput, ActionInputKind, ActionStepKind, PortalAction, PortalActionCategory, PortalActionInput } from '../../../api/portalActions/client';
import { useTeams } from '../../settings/queries';
import { ToolLogo } from './common';
import { useDeletePortalAction, usePortalAction, useSavePortalAction } from './queries';
import './actions.css';

type Draft = {
  name: string;
  description: string;
  category: PortalActionCategory;
  blueprint: string;
  ownerTeam: string;
  allowedTeams: string;
  inputs: ActionInput[];
  kind: ActionStepKind;
  requests: ActionHttpRequest[];
  fields: Record<string, string>;
};

const EMPTY_REQUEST: ActionHttpRequest = { method: 'POST', url: 'https://', headers: [], body: '' };
const KINDS: ActionInputKind[] = ['text', 'select', 'number', 'boolean'];

function fromAction(a: PortalAction): Draft {
  return {
    name: a.name,
    description: a.description,
    category: a.category,
    blueprint: a.blueprint,
    ownerTeam: a.ownerTeam,
    allowedTeams: a.allowedTeams.join(', '),
    inputs: a.inputs,
    kind: a.step.kind,
    requests: a.step.requests?.length ? a.step.requests : [EMPTY_REQUEST],
    fields: a.step.fields ?? {},
  };
}

function initialDraft(blueprintKey: string | null): Draft {
  const b = findBlueprint(blueprintKey);
  if (b) {
    return {
      name: b.title,
      description: b.description,
      category: b.category,
      blueprint: b.key,
      ownerTeam: '',
      allowedTeams: '',
      inputs: b.inputs,
      kind: b.step.kind,
      requests: b.step.requests?.length ? b.step.requests : [EMPTY_REQUEST],
      fields: b.step.fields ?? {},
    };
  }
  return { name: '', description: '', category: 'other', blueprint: '', ownerTeam: '', allowedTeams: '', inputs: [], kind: 'http', requests: [EMPTY_REQUEST], fields: {} };
}

function toInput(d: Draft, secrets: Record<string, string | null>): PortalActionInput {
  const http = d.kind === 'http';
  return {
    name: d.name,
    description: d.description,
    category: d.category,
    blueprint: d.blueprint,
    ownerTeam: d.ownerTeam,
    allowedTeams: d.allowedTeams
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean),
    inputs: d.inputs.map((i) => (i.kind === 'select' ? { ...i, options: (i.options ?? []).map((o) => o.trim()).filter(Boolean) } : { ...i, options: [] })),
    step: { kind: d.kind, requests: http ? d.requests : [], fields: http ? {} : Object.fromEntries(STEP_FIELDS[d.kind].map(([f]) => [f, d.fields[f] ?? ''])) },
    // Secrets added but left empty are not sent.
    secrets: http ? Object.fromEntries(Object.entries(secrets).filter(([, v]) => v === null || v.trim() !== '')) : {},
  };
}

/** Creates or edits a self-service action: form inputs, the step and its credentials. */
export function ActionEditorPage() {
  const { id } = useParams();
  const [params] = useSearchParams();
  const existing = usePortalAction(id);
  const [draft, setDraft] = useState<Draft>(() => initialDraft(params.get('blueprint')));
  const [loaded, setLoaded] = useState(!id);
  const [secrets, setSecrets] = useState<Record<string, string | null>>({});
  const [newSecret, setNewSecret] = useState('');
  const teams = useTeams({ limit: 200, offset: 0, sort: 'name' });
  const save = useSavePortalAction();
  const remove = useDeletePortalAction();
  const navigate = useNavigate();

  useEffect(() => {
    if (id && existing.data && !loaded) {
      setDraft(fromAction(existing.data));
      setLoaded(true);
    }
  }, [id, existing.data, loaded]);

  const set = <K extends keyof Draft>(k: K, v: Draft[K]) => setDraft((d) => ({ ...d, [k]: v }));
  const setInput = (i: number, patch: Partial<ActionInput>) =>
    set(
      'inputs',
      draft.inputs.map((x, j) => (j === i ? { ...x, ...patch } : x)),
    );
  const moveInput = (i: number, by: number) => {
    const next = [...draft.inputs];
    const [x] = next.splice(i, 1);
    next.splice(i + by, 0, x);
    set('inputs', next);
  };
  const setRequest = (i: number, patch: Partial<ActionHttpRequest>) =>
    set(
      'requests',
      draft.requests.map((r, j) => (j === i ? { ...r, ...patch } : r)),
    );

  const stored = existing.data?.secretNames ?? [];
  const used = draft.kind === 'http' ? secretsUsed({ kind: 'http', requests: draft.requests }) : [];
  const secretNames = [...new Set([...stored, ...used, ...Object.keys(secrets)])].sort();
  const bp = findBlueprint(draft.blueprint);

  if (id && existing.isLoading) return <div className="psa-page psa-muted">Loading…</div>;
  if (id && existing.isError) return <div className="psa-page form-error">{(existing.error as Error).message}</div>;

  const onSave = () =>
    save.mutate(
      { id, input: toInput(draft, secrets) },
      {
        onSuccess: (a) => navigate(`/portal/actions/${a.id}`),
      },
    );

  return (
    <div className="psa-page psa-editor">
      <Link className="psa-back" to={id ? `/portal/actions/${id}` : '/portal/actions'}>
        <ArrowLeft size={14} /> {id ? 'Back to the action' : 'Self-Service Actions'}
      </Link>
      <h1 className="psa-title psa-title-row">
        {bp && <ToolLogo logo={bp.logo} size={24} />}
        {id ? `Edit ${existing.data?.name ?? 'action'}` : bp ? `New action from “${bp.title}”` : 'New action'}
      </h1>

      <section className="psa-panel">
        <h2>Details</h2>
        <div className="psa-grid-2">
          <label className="sc-field">
            <span>Name</span>
            <input className="sc-input" value={draft.name} onChange={(e) => set('name', e.target.value)} placeholder="Scaffold a storefront service" />
          </label>
          <label className="sc-field">
            <span>Category</span>
            <select className="sc-input" value={draft.category} onChange={(e) => set('category', e.target.value as PortalActionCategory)}>
              {(Object.keys(CATEGORY_LABEL) as PortalActionCategory[]).map((c) => (
                <option key={c} value={c}>
                  {CATEGORY_LABEL[c]}
                </option>
              ))}
            </select>
          </label>
          <label className="sc-field psa-span-2">
            <span>Description</span>
            <textarea className="sc-input" rows={2} value={draft.description} onChange={(e) => set('description', e.target.value)} />
          </label>
          <label className="sc-field">
            <span>Owner team</span>
            <input className="sc-input" list="psa-teams" value={draft.ownerTeam} onChange={(e) => set('ownerTeam', e.target.value)} placeholder="Team handle" />
          </label>
          <label className="sc-field">
            <span>Who may run it</span>
            <input className="sc-input" list="psa-teams" value={draft.allowedTeams} onChange={(e) => set('allowedTeams', e.target.value)} placeholder="Every standard user and admin" />
            <small className="psa-muted">Team handles, comma-separated. Admins may always run it.</small>
          </label>
          <datalist id="psa-teams">
            {teams.data?.teams.map((t) => (
              <option key={t.handle} value={t.handle}>
                {t.name}
              </option>
            ))}
          </datalist>
        </div>
      </section>

      <section className="psa-panel">
        <div className="psa-section-head">
          <h2>Form inputs</h2>
          <button
            className="btn btn-sm"
            onClick={() => set('inputs', [...draft.inputs, { key: `field_${draft.inputs.length + 1}`, label: '', kind: 'text', required: false, options: [], default: '', description: '' }])}
          >
            <Plus size={13} /> Add input
          </button>
        </div>
        {draft.inputs.length === 0 && <p className="psa-muted">No inputs: the action runs with fixed values.</p>}
        {draft.inputs.map((inp, i) => (
          <div key={i} className="psa-input-row">
            <input className="sc-input" aria-label="Key" value={inp.key} onChange={(e) => setInput(i, { key: e.target.value })} placeholder="key" />
            <input className="sc-input grow" aria-label="Label" value={inp.label} onChange={(e) => setInput(i, { label: e.target.value })} placeholder="Label" />
            <select className="sc-input" aria-label="Kind" value={inp.kind} onChange={(e) => setInput(i, { kind: e.target.value as ActionInputKind })}>
              {KINDS.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
            {inp.kind === 'select' && (
              <input
                className="sc-input grow"
                aria-label="Options"
                value={(inp.options ?? []).join(', ')}
                onChange={(e) => setInput(i, { options: e.target.value.split(',').map((o) => o.trimStart()) })}
                placeholder="Options, comma-separated"
              />
            )}
            <input className="sc-input" aria-label="Default" value={inp.default ?? ''} onChange={(e) => setInput(i, { default: e.target.value })} placeholder="Default" />
            <label className="psa-inline">
              <input type="checkbox" checked={!!inp.required} onChange={(e) => setInput(i, { required: e.target.checked })} /> Required
            </label>
            <button className="sc-icon-btn" aria-label="Move up" disabled={i === 0} onClick={() => moveInput(i, -1)}>
              <ArrowUp size={14} />
            </button>
            <button className="sc-icon-btn" aria-label="Move down" disabled={i === draft.inputs.length - 1} onClick={() => moveInput(i, 1)}>
              <ArrowDown size={14} />
            </button>
            <button
              className="sc-icon-btn"
              aria-label="Remove input"
              onClick={() =>
                set(
                  'inputs',
                  draft.inputs.filter((_, j) => j !== i),
                )
              }
            >
              <Trash2 size={14} />
            </button>
          </div>
        ))}
      </section>

      <section className="psa-panel">
        <h2>Step</h2>
        <label className="sc-field psa-narrow">
          <span>What the action does</span>
          <select className="sc-input" value={draft.kind} onChange={(e) => set('kind', e.target.value as ActionStepKind)}>
            {(Object.keys(STEP_LABEL) as ActionStepKind[]).map((k) => (
              <option key={k} value={k}>
                {STEP_LABEL[k]}
              </option>
            ))}
          </select>
        </label>
        <p className="psa-help">
          Templates use <code>{'{{inputs.key}}'}</code>, <code>{'{{user.email}}'}</code>, <code>{'{{user.name}}'}</code>
          {draft.kind === 'http' && (
            <>
              , <code>{'{{secrets.name}}'}</code> and, from the second request on, <code>{'{{steps.1.field}}'}</code> (a value of request 1's JSON response). Filters: <code>{'| base64'}</code>,{' '}
              <code>{'| path'}</code>. Values are percent-encoded in URLs and stay inside the strings of the JSON body; the URL's host is written out
            </>
          )}
          .
        </p>
        {draft.kind === 'http' ? (
          <>
            {draft.requests.map((r, i) => (
              <div key={i} className="psa-request">
                <div className="psa-request-head">
                  <strong>Request {i + 1}</strong>
                  {draft.requests.length > 1 && (
                    <button
                      className="sc-icon-btn"
                      aria-label="Remove request"
                      onClick={() =>
                        set(
                          'requests',
                          draft.requests.filter((_, j) => j !== i),
                        )
                      }
                    >
                      <Trash2 size={14} />
                    </button>
                  )}
                </div>
                <div className="psa-url-row">
                  <select className="sc-input" aria-label="Method" value={r.method} onChange={(e) => setRequest(i, { method: e.target.value })}>
                    {METHODS.map((m) => (
                      <option key={m}>{m}</option>
                    ))}
                  </select>
                  <input className="sc-input grow psa-mono" aria-label="URL" value={r.url} onChange={(e) => setRequest(i, { url: e.target.value })} />
                </div>
                <div className="sc-field">
                  <span>Headers</span>
                  {(r.headers ?? []).map((h, j) => (
                    <div key={j} className="psa-url-row">
                      <input
                        className="sc-input"
                        aria-label="Header name"
                        value={h.name}
                        onChange={(e) => setRequest(i, { headers: (r.headers ?? []).map((x, k) => (k === j ? { ...x, name: e.target.value } : x)) })}
                      />
                      <input
                        className="sc-input grow psa-mono"
                        aria-label="Header value"
                        value={h.value}
                        onChange={(e) => setRequest(i, { headers: (r.headers ?? []).map((x, k) => (k === j ? { ...x, value: e.target.value } : x)) })}
                      />
                      <button className="sc-icon-btn" aria-label="Remove header" onClick={() => setRequest(i, { headers: (r.headers ?? []).filter((_, k) => k !== j) })}>
                        <Trash2 size={14} />
                      </button>
                    </div>
                  ))}
                  <button className="sc-link psa-add" onClick={() => setRequest(i, { headers: [...(r.headers ?? []), { name: '', value: '' }] })}>
                    + Add header
                  </button>
                </div>
                <label className="sc-field">
                  <span>JSON body</span>
                  <textarea className="sc-input psa-mono" rows={5} value={r.body ?? ''} onChange={(e) => setRequest(i, { body: e.target.value })} placeholder='{"name": "{{inputs.name}}"}' />
                </label>
              </div>
            ))}
            {draft.requests.length < 5 && (
              <button className="btn btn-sm" onClick={() => set('requests', [...draft.requests, { ...EMPTY_REQUEST }])}>
                <Plus size={13} /> Add a request
              </button>
            )}
          </>
        ) : (
          <div className="psa-grid-2">
            {STEP_FIELDS[draft.kind].map(([f, required]) => (
              <label key={f} className="sc-field">
                <span>
                  {f}
                  {required && <span className="psa-required"> *</span>}
                </span>
                <input className="sc-input psa-mono" value={draft.fields[f] ?? ''} onChange={(e) => set('fields', { ...draft.fields, [f]: e.target.value })} />
              </label>
            ))}
          </div>
        )}
      </section>

      {draft.kind === 'http' && (
        <section className="psa-panel">
          <h2>Secrets</h2>
          <p className="psa-help">Stored encrypted on the server and never shown again. Runs redact them from responses.</p>
          {secretNames.length === 0 && <p className="psa-muted">This action uses no secrets.</p>}
          {secretNames.map((name) => {
            const isStored = stored.includes(name);
            const change = secrets[name];
            const state = change === null ? 'will be removed' : change ? (isStored ? 'will be replaced' : 'will be set') : isStored ? 'stored' : 'not set';
            const hint = bp?.secrets.find((s) => s.name === name)?.hint;
            return (
              <div key={name} className="psa-secret-row">
                <code>{name}</code>
                <span className={`psa-secret-state ${state === 'not set' ? 'warn' : ''}`}>{state}</span>
                <input
                  className="sc-input grow"
                  type="password"
                  autoComplete="new-password"
                  aria-label={`Value of ${name}`}
                  placeholder={hint ?? (isStored ? 'Enter a new value to replace it' : 'Value')}
                  value={change ?? ''}
                  onChange={(e) => setSecrets((s) => ({ ...s, [name]: e.target.value }))}
                />
                {(isStored || change !== undefined) && (
                  <button
                    className="btn btn-sm"
                    onClick={() =>
                      setSecrets((s) => {
                        const next = { ...s };
                        if (isStored) next[name] = null;
                        else delete next[name];
                        return next;
                      })
                    }
                  >
                    Remove
                  </button>
                )}
              </div>
            );
          })}
          <div className="psa-secret-row">
            <input className="sc-input" value={newSecret} onChange={(e) => setNewSecret(e.target.value)} placeholder="new_secret_name" aria-label="New secret name" />
            <button
              className="btn btn-sm"
              disabled={!newSecret.trim()}
              onClick={() => {
                setSecrets((s) => ({ ...s, [newSecret.trim()]: '' }));
                setNewSecret('');
              }}
            >
              <Plus size={13} /> Add secret
            </button>
          </div>
        </section>
      )}

      {save.isError && <div className="form-error">{(save.error as Error).message}</div>}
      <div className="psa-editor-actions">
        {id && (
          <button
            className="btn btn-danger"
            disabled={remove.isPending}
            onClick={() => {
              if (window.confirm(`Delete ${draft.name}? Its run history goes too.`)) remove.mutate(id, { onSuccess: () => navigate('/portal/actions') });
            }}
          >
            <Trash2 size={14} /> Delete
          </button>
        )}
        <span className="psa-grow" />
        <Link className="btn" to={id ? `/portal/actions/${id}` : '/portal/actions'}>
          Cancel
        </Link>
        <button className="btn btn-primary" disabled={save.isPending} onClick={onSave}>
          {save.isPending ? 'Saving…' : id ? 'Save' : 'Create action'}
        </button>
      </div>
    </div>
  );
}
