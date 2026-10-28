import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { AlertTriangle, Check, ChevronDown, ChevronRight, Copy, Flag, Info, Pencil, Plus, Trash2 } from 'lucide-react';
import type { FeatureFlag, FlagLifecycle, FlagSdkAccess } from '../../api/featureFlags/client';
import {
  FLAG_TYPES,
  TYPE_LABEL,
  draftFromFlag,
  draftKey,
  draftProblems,
  inputFromDraft,
  newFlagDraft,
  newVariantDraft,
  parseVariantValue,
  validKey,
  variantColor,
  variantProblems,
  withFlagType,
  type FlagDraft,
  type VariantDraft,
} from '../../api/featureFlags/logic';
import { searchTeams } from '../../api/identity/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Step } from '../monitors/MonitorEditor';
import { ChipsInput, Swatch } from './common';
import { useFeatureFlag, useSaveFlag } from './queries';
import '../monitors/monitors.css';
import './featureFlags.css';

const SDK_OPTIONS: { value: FlagSdkAccess; label: string }[] = [
  { value: 'all', label: 'Client and Server' },
  { value: 'client', label: 'Client only' },
  { value: 'server', label: 'Server only' },
];

const LIFECYCLES: { value: FlagLifecycle; label: string; hint: string }[] = [
  { value: 'temporary', label: 'Temporary', hint: 'A release or an experiment: remove it from the code once it is fully rolled out.' },
  { value: 'permanent', label: 'Permanent', hint: 'An operational switch or a setting that stays.' },
];

/** Create Flag (and Edit): details, variants and approvals in numbered steps. */
export function CreateFlagPage() {
  const { id } = useParams();
  const q = useFeatureFlag(id);
  if (id && q.isPending) return <Loading label="Loading flag…" />;
  if (id && q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <FlagForm key={id ?? 'new'} flag={q.data} />;
}

function FlagForm({ flag }: { flag?: FeatureFlag }) {
  const navigate = useNavigate();
  const [d, setD] = useState<FlagDraft>(() => (flag ? draftFromFlag(flag) : newFlagDraft()));
  const [editingKey, setEditingKey] = useState(false);
  const [describing, setDescribing] = useState(() => !!flag?.description);
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set(d.variants.map((v) => v.id)));
  const save = useSaveFlag(flag?.id);
  const teams = useQuery({ queryKey: ['teams', 'handles'], queryFn: () => searchTeams({ limit: 200, offset: 0, sort: 'name' }), staleTime: 300_000, retry: 0 });
  const teamNames = (teams.data?.teams ?? []).map((t) => t.handle);
  const set = (p: Partial<FlagDraft>) => setD((x) => ({ ...x, ...p }));
  const setVariant = (vid: number, p: Partial<VariantDraft>) => setD((x) => ({ ...x, variants: x.variants.map((v) => (v.id === vid ? { ...v, ...p } : v)) }));

  const missing = draftProblems(d);
  const problems = variantProblems(d);
  const key = draftKey(d);
  const fixedVariants = d.flagType === 'boolean';
  const submit = () => save.mutate({ ...inputFromDraft(d), version: flag?.version }, { onSuccess: (f) => navigate(`/feature-flags/${f.id}`) });
  const addVariant = () => {
    const v = newVariantDraft(d);
    setD((x) => ({ ...x, variants: [...x.variants, v] }));
    setExpanded((s) => new Set([...s, v.id]));
  };
  const duplicate = (v: VariantDraft) => {
    const copy = { ...newVariantDraft(d), text: v.text, key: `${v.key}-copy` };
    setD((x) => ({ ...x, variants: [...x.variants, copy] }));
    setExpanded((s) => new Set([...s, copy.id]));
  };
  const remove = (i: number) =>
    setD((x) => ({ ...x, variants: x.variants.filter((_, j) => j !== i), defaultIndex: x.defaultIndex === i ? 0 : x.defaultIndex > i ? x.defaultIndex - 1 : x.defaultIndex }));
  const allOpen = d.variants.every((v) => expanded.has(v.id));

  return (
    <div className="mon-editor">
      <header className="mon-crumbs">
        <Link to="/feature-flags" className="mon-crumb-root">
          <Flag size={22} /> Feature Flags
        </Link>
        <ChevronRight size={18} className="muted" />
        {flag ? (
          <>
            <Link to={`/feature-flags/${flag.id}`} className="mon-crumb-link">
              {flag.name}
            </Link>
            <ChevronRight size={18} className="muted" />
            <span>Edit</span>
          </>
        ) : (
          <span>Create Flag</span>
        )}
      </header>
      <div className="mon-editor-scroll">
        <div className="mon-steps ff-editor-grid">
          <Step n={1} title="Flag Details" done={!missing.includes(1)}>
            <div className="ff-details">
              <div>
                <label className="ff-field">
                  <span>
                    Flag name <em>*</em>
                  </span>
                  <input autoFocus={!flag} value={d.name} placeholder="New payment flow" onChange={(e) => set({ name: e.target.value })} maxLength={200} />
                </label>
                {describing ? (
                  <label className="ff-field">
                    <span>Flag description</span>
                    <textarea value={d.description} placeholder="What the flag changes and when it can be removed" onChange={(e) => set({ description: e.target.value })} maxLength={2000} />
                  </label>
                ) : (
                  <div className="ff-field">
                    <button type="button" className="ff-add-desc" onClick={() => setDescribing(true)}>
                      Add a Description <Pencil size={14} />
                    </button>
                  </div>
                )}
                <div className="ff-field">
                  <span className="ff-label">Team</span>
                  <ChipsInput label="Teams" values={d.teams} onChange={(teams) => set({ teams })} placeholder="Select or type a team" suggestions={teamNames} />
                </div>
                <div className="ff-field">
                  <span className="ff-label">Tags</span>
                  <ChipsInput label="Tags" values={d.tags} onChange={(tags) => set({ tags })} placeholder="service:storefront-api" />
                </div>
                <div className="ff-question">
                  <span className="ff-label">Which SDKs can request this flag?</span>
                  <div className="ff-radios ff-radios-row" role="radiogroup" aria-label="SDK access">
                    {SDK_OPTIONS.map((o) => (
                      <label key={o.value}>
                        <input type="radio" name="sdk" checked={d.sdkAccess === o.value} onChange={() => set({ sdkAccess: o.value })} /> {o.label}
                      </label>
                    ))}
                  </div>
                  {d.sdkAccess !== 'server' && (
                    <div className="ff-note">
                      <Info size={15} /> Client SDKs run in the browser or on devices: anyone using the app can read every variant’s value. Keep secrets in server-only flags.
                    </div>
                  )}
                </div>
                <div className="ff-question">
                  <span className="ff-label">Is this a temporary or permanent flag?</span>
                  <div className="ff-radios" role="radiogroup" aria-label="Lifecycle">
                    {LIFECYCLES.map((o) => (
                      <label key={o.value} title={o.hint}>
                        <input type="radio" name="lifecycle" checked={d.lifecycle === o.value} onChange={() => set({ lifecycle: o.value })} /> {o.label}
                      </label>
                    ))}
                  </div>
                  <p className="ff-hint">{LIFECYCLES.find((o) => o.value === d.lifecycle)?.hint}</p>
                </div>
              </div>
              <div>
                <div className="ff-field">
                  <span className="ff-label">Flag key</span>
                  <small>Used in your codebase with the OpenFeature SDK</small>
                  <div className="ff-keybox">
                    {flag ? (
                      <span>{flag.key}</span>
                    ) : editingKey ? (
                      <input
                        aria-label="Flag key"
                        autoFocus
                        value={d.key || key}
                        onChange={(e) => set({ key: e.target.value.toLowerCase() })}
                        onBlur={() => setEditingKey(false)}
                        onKeyDown={(e) => e.key === 'Enter' && setEditingKey(false)}
                      />
                    ) : (
                      <>
                        {key ? <span>{key}</span> : <span className="muted">Derived from the flag name</span>}
                        <button type="button" className="icon-plain" title="Edit the key" onClick={() => setEditingKey(true)}>
                          <Pencil size={14} />
                        </button>
                      </>
                    )}
                  </div>
                  {!flag && key && !validKey(key) && <span className="ff-error-text">Lowercase letters, digits, '-', '_' and '.', starting with a letter or digit (at most 100).</span>}
                  {flag && <small>A flag’s key can’t be changed once created.</small>}
                </div>
              </div>
            </div>
          </Step>
          <Step n={2} title="Variants" done={!missing.includes(2)}>
            <div className="ff-seg" role="radiogroup" aria-label="Flag type">
              {FLAG_TYPES.map((t) => (
                <button
                  key={t}
                  role="radio"
                  aria-checked={d.flagType === t}
                  className={d.flagType === t ? 'active' : ''}
                  disabled={!!flag}
                  title={flag ? 'A flag’s type can’t be changed' : undefined}
                  onClick={() => {
                    const next = withFlagType(d, t);
                    setD(next);
                    setExpanded(new Set(next.variants.map((v) => v.id)));
                  }}
                >
                  {TYPE_LABEL[t]}
                </button>
              ))}
            </div>
            <div className="ff-variants-head">
              <span className="ff-label">
                {d.variants.length} variant{d.variants.length === 1 ? '' : 's'}
              </span>
              <button type="button" className="link-btn" onClick={() => setExpanded(allOpen ? new Set() : new Set(d.variants.map((v) => v.id)))}>
                {allOpen ? 'Collapse All' : 'Expand All'}
              </button>
            </div>
            {d.variants.map((v, i) => {
              const open = expanded.has(v.id);
              const parsed = parseVariantValue(d.flagType, v.text);
              return (
                <div key={v.id} className="ff-variant">
                  <div className="ff-variant-head">
                    <button
                      type="button"
                      className="ff-toggle"
                      aria-label={open ? 'Collapse' : 'Expand'}
                      onClick={() => setExpanded((s) => (s.has(v.id) ? new Set([...s].filter((x) => x !== v.id)) : new Set([...s, v.id])))}
                    >
                      {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                    </button>
                    <Swatch color={variantColor(i)} />
                    <span className="ff-variant-name">{v.key || <span className="muted">Unnamed</span>}</span>
                    {!open && <span className="ff-variant-value">{parsed.error ? v.text : JSON.stringify(parsed.value)}</span>}
                    <span className="ff-variant-tools">
                      {d.defaultIndex === i ? (
                        <span className="ff-default-label">Default</span>
                      ) : (
                        <button type="button" className="link-btn" onClick={() => set({ defaultIndex: i })}>
                          Make Default
                        </button>
                      )}
                      {!fixedVariants && (
                        <>
                          <button type="button" className="icon-plain" title="Duplicate" onClick={() => duplicate(v)}>
                            <Copy size={15} />
                          </button>
                          <button type="button" className="icon-plain" title="Remove" disabled={d.variants.length <= 1} onClick={() => remove(i)}>
                            <Trash2 size={15} />
                          </button>
                        </>
                      )}
                    </span>
                  </div>
                  {open && (
                    <div className="ff-variant-body">
                      <label className="ff-field">
                        <span>Name</span>
                        <input value={v.key} disabled={fixedVariants} onChange={(e) => setVariant(v.id, { key: e.target.value })} maxLength={100} />
                      </label>
                      <label className="ff-field">
                        <span>Value</span>
                        {d.flagType === 'json' ? (
                          <textarea rows={4} value={v.text} onChange={(e) => setVariant(v.id, { text: e.target.value })} spellCheck={false} />
                        ) : (
                          <input value={v.text} disabled={fixedVariants} inputMode={d.flagType === 'string' ? undefined : 'decimal'} onChange={(e) => setVariant(v.id, { text: e.target.value })} />
                        )}
                      </label>
                      {problems.get(v.id) && <span className="ff-error-text">{problems.get(v.id)}</span>}
                    </div>
                  )}
                </div>
              );
            })}
            {!fixedVariants && (
              <button type="button" className="btn btn-sm" onClick={addVariant} disabled={d.variants.length >= 20}>
                <Plus size={14} /> Add Variant
              </button>
            )}
            <p className="ff-hint">New environments serve the default variant until you change their targeting.</p>
          </Step>
          <Step n={3} title="Approvals & notifications (optional)" done last>
            <p className="ff-soon-step">
              Every change is recorded in the flag’s history and in the audit trail. Required approvals and change notifications aren’t available yet; environments can already be marked as requiring
              approval in Settings.
            </p>
          </Step>
        </div>
      </div>
      <footer className="mon-editor-foot">
        {missing.length ? (
          <span className="mon-foot-warn">
            <AlertTriangle size={20} /> Complete step{missing.length > 1 ? 's' : ''} {missing.join(' and ')}
          </span>
        ) : (
          <span className="mon-foot-ok">
            <Check size={18} /> Ready to {flag ? 'save' : 'create'}
          </span>
        )}
        {save.isError && <span className="form-error">{(save.error as Error).message}</span>}
        <span className="mon-foot-actions">
          <button className="btn" onClick={() => navigate(flag ? `/feature-flags/${flag.id}` : '/feature-flags')}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={missing.length > 0 || save.isPending} onClick={submit}>
            {flag ? 'Save Flag' : 'Create Flag'}
          </button>
        </span>
      </footer>
    </div>
  );
}
