import { useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ChevronLeft, Plus, Trash2 } from 'lucide-react';
import type { CatalogType } from '../../api/catalog/client';
import { operatorArity, OPERATOR_LABELS, validateTarget } from '../../api/policies/logic';
import type { PolicyCategory, PolicyCondition, PolicyOperator, ResourcePolicyInput } from '../../api/policies/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { TYPE_LABELS } from './catalogModel';
import { CATEGORY_NAMES, PRIORITIES } from './policyModel';
import { PriorityBadge, ScoreBar } from './PoliciesPage';
import { ResourceLogo } from './ResourceLogo';
import { useDebouncedValue } from './useDebouncedValue';
import { usePolicies, usePolicy, usePolicyAttributes, usePolicyPreview, useSavePolicy } from './queries';
import './policies.css';

interface Draft {
  name: string;
  description: string;
  priority: number;
  category: PolicyCategory;
  resourceType: CatalogType;
  scope: string;
  /** Values as typed: one per line / comma separated for lists. */
  conditions: { attribute: string; operator: PolicyOperator; values: string }[];
}

const EMPTY: Draft = { name: '', description: '', priority: 2, category: 'reliability', resourceType: 'kube_deployment', scope: '', conditions: [{ attribute: '', operator: 'exists', values: '' }] };
const OPERATORS = Object.keys(OPERATOR_LABELS) as PolicyOperator[];

function toConditions(d: Draft): PolicyCondition[] {
  return d.conditions.map((c) => ({
    attribute: c.attribute.trim(),
    operator: c.operator,
    values:
      operatorArity(c.operator) === 'none'
        ? []
        : operatorArity(c.operator) === 'one'
          ? [c.values.trim()].filter(Boolean)
          : c.values
              .split(',')
              .map((v) => v.trim())
              .filter(Boolean),
  }));
}

function fromConditions(list: PolicyCondition[]): Draft['conditions'] {
  return list.map((c) => ({ attribute: c.attribute, operator: c.operator, values: (c.values ?? []).join(', ') }));
}

/** Creates or edits a policy (`/infrastructure/policies/new[?template=…]`, `/infrastructure/policies/:id/edit`), with its live preview score. */
export function PolicyEditorPage() {
  const { id } = useParams();
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const templateKey = params.get('template');
  const existing = usePolicy(id ?? '', '');
  const list = usePolicies('');
  const attributes = usePolicyAttributes();
  const save = useSavePolicy(id);
  const [draft, setDraft] = useState<Draft | null>(id || templateKey ? null : EMPTY);

  // Start from the stored policy or the chosen template once it has loaded.
  useEffect(() => {
    if (draft) return;
    if (id && existing.data) {
      const p = existing.data.policy;
      setDraft({ name: p.name, description: p.description, priority: p.priority, category: p.category, resourceType: p.resourceType, scope: p.scope, conditions: fromConditions(p.conditions) });
    } else if (!id && templateKey && list.data) {
      const t = list.data.templates.find((x) => x.key === templateKey);
      setDraft(
        t
          ? {
              name: t.name,
              description: t.description,
              priority: t.priority,
              category: t.category,
              resourceType: t.target.resourceType,
              scope: t.target.scope ?? '',
              conditions: fromConditions(t.target.conditions),
            }
          : EMPTY,
      );
    }
  }, [draft, id, existing.data, templateKey, list.data]);

  const target = draft ? { resourceType: draft.resourceType, scope: draft.scope, conditions: toConditions(draft) } : null;
  const problem = target ? validateTarget(target) : null;
  const debounced = useDebouncedValue(target && !problem ? target : null, 400);
  const preview = usePolicyPreview(debounced);
  const typeAttrs = attributes.data?.find((a) => a.resourceType === draft?.resourceType);
  const typeOptions = useMemo(() => {
    const counts = new Map((attributes.data ?? []).map((a) => [a.resourceType, a.resources]));
    return (Object.keys(TYPE_LABELS) as CatalogType[]).map((t) => ({
      value: t,
      label: TYPE_LABELS[t],
      icon: <ResourceLogo type={t} size={14} />,
      hint: counts.get(t) ? counts.get(t)!.toLocaleString() : undefined,
    }));
  }, [attributes.data]);

  if (id && existing.isError) return <ErrorState error={existing.error} onRetry={() => existing.refetch()} />;
  if (!draft) return <Loading label="Loading policy…" />;

  const set = (patch: Partial<Draft>) => setDraft({ ...draft, ...patch });
  const setCond = (i: number, patch: Partial<Draft['conditions'][number]>) => set({ conditions: draft.conditions.map((c, j) => (j === i ? { ...c, ...patch } : c)) });
  const submit = () => {
    const input: ResourcePolicyInput = { ...draft, conditions: toConditions(draft), template: id ? null : templateKey };
    save.mutate(input, { onSuccess: (p) => navigate(`/infrastructure/policies/${p.id}`) });
  };
  const canSave = !!draft.name.trim() && !problem && !save.isPending;
  const e = preview.data?.evaluation;

  return (
    <div className="rc-page">
      <div className="hosts-scroll pol-editor">
        <Link to={id ? `/infrastructure/policies/${id}` : '/infrastructure/policies'} className="pol-back">
          <ChevronLeft size={14} /> {id ? 'Back to policy' : 'Policies'}
        </Link>
        <h1 className="pol-title">{id ? 'Edit Policy' : 'Create a Policy'}</h1>
        <div className="pol-editor-grid">
          <div className="pol-form">
            <section className="pol-step">
              <h3>
                <span className="pol-step-n">1</span> Choose a resource type
              </h3>
              <Select label="Resource type" value={draft.resourceType} options={typeOptions} onChange={(t) => set({ resourceType: t })} />
              {attributes.data && !typeAttrs && <p className="pol-hint">No {TYPE_LABELS[draft.resourceType]} resources in the catalog yet; the policy will score once some report.</p>}
            </section>

            <section className="pol-step">
              <h3>
                <span className="pol-step-n">2</span> Narrow the scope <span className="muted-text">(optional)</span>
              </h3>
              <input className="pol-input mono" value={draft.scope} placeholder="env:prod cluster:shop-* -team:sandbox" onChange={(ev) => set({ scope: ev.target.value })} />
              <p className="pol-hint">Catalog search syntax: fields, tags and attributes, with * wildcards and - to exclude.</p>
            </section>

            <section className="pol-step">
              <h3>
                <span className="pol-step-n">3</span> Define the rule
              </h3>
              <p className="pol-hint">A resource complies when every condition holds. For attributes with several values (image tags, tags), every value must satisfy it.</p>
              <datalist id="pol-attributes">
                {typeAttrs?.attributes.map((a) => (
                  <option key={a.key} value={a.key}>
                    {`${a.count} of ${typeAttrs.resources} · e.g. ${a.examples.slice(0, 3).join(', ')}`}
                  </option>
                ))}
              </datalist>
              {draft.conditions.map((c, i) => {
                const known = typeAttrs?.attributes.find((a) => a.key === c.attribute.trim());
                return (
                  <div key={i} className="pol-cond">
                    <span className="pol-cond-join">{i === 0 ? 'Where' : 'and'}</span>
                    <input className="pol-input mono" list="pol-attributes" placeholder="attribute (e.g. replicas)" value={c.attribute} onChange={(ev) => setCond(i, { attribute: ev.target.value })} />
                    <Select value={c.operator} options={OPERATORS.map((o) => ({ value: o, label: OPERATOR_LABELS[o] }))} onChange={(o) => setCond(i, { operator: o })} />
                    {operatorArity(c.operator) !== 'none' ? (
                      <input
                        className="pol-input mono"
                        placeholder={operatorArity(c.operator) === 'many' ? 'value, value, …' : c.operator.startsWith('version') ? '1.31' : 'value'}
                        value={c.values}
                        onChange={(ev) => setCond(i, { values: ev.target.value })}
                      />
                    ) : (
                      <span />
                    )}
                    <button className="icon-btn" title="Remove condition" disabled={draft.conditions.length === 1} onClick={() => set({ conditions: draft.conditions.filter((_, j) => j !== i) })}>
                      <Trash2 size={14} />
                    </button>
                    {c.attribute.trim() && typeAttrs && (
                      <span className="pol-cond-note">
                        {known
                          ? `Set on ${known.count.toLocaleString()} of ${typeAttrs.resources.toLocaleString()} resources · e.g. ${known.examples.slice(0, 3).join(', ')}`
                          : 'No resource of this type has this attribute yet.'}
                      </span>
                    )}
                  </div>
                );
              })}
              <button className="link-btn" disabled={draft.conditions.length >= 10} onClick={() => set({ conditions: [...draft.conditions, { attribute: '', operator: 'exists', values: '' }] })}>
                <Plus size={14} /> Add condition
              </button>
              {problem && draft.conditions.some((c) => c.attribute.trim()) && <p className="pol-error">{problem}</p>}
            </section>

            <section className="pol-step">
              <h3>
                <span className="pol-step-n">4</span> Name and classify
              </h3>
              <label className="pol-field">
                <span>Name</span>
                <input className="pol-input" value={draft.name} maxLength={200} placeholder="Production deployments run at least two replicas" onChange={(ev) => set({ name: ev.target.value })} />
              </label>
              <div className="pol-row">
                <Select
                  label="Priority"
                  value={String(draft.priority)}
                  options={PRIORITIES.map((p) => ({ value: String(p), label: <PriorityBadge priority={p} /> }))}
                  onChange={(p) => set({ priority: Number(p) })}
                />
                <Select
                  label="Category"
                  value={draft.category}
                  options={(Object.keys(CATEGORY_NAMES) as PolicyCategory[]).map((c) => ({ value: c, label: CATEGORY_NAMES[c] }))}
                  onChange={(c) => set({ category: c })}
                />
              </div>
              <label className="pol-field">
                <span>Description</span>
                <textarea
                  className="pol-input"
                  rows={3}
                  maxLength={2000}
                  value={draft.description}
                  placeholder="Why this matters and how to fix a noncompliant resource."
                  onChange={(ev) => set({ description: ev.target.value })}
                />
              </label>
            </section>

            {save.isError && <p className="pol-error">{(save.error as Error).message}</p>}
            <div className="pol-form-actions">
              <button className="btn" onClick={() => navigate(-1)}>
                Cancel
              </button>
              <button className="btn btn-primary" disabled={!canSave} onClick={submit}>
                {id ? 'Save Policy' : 'Create Policy'}
              </button>
            </div>
          </div>

          <aside className="pol-preview" aria-live="polite">
            <h3>Preview Score</h3>
            {problem ? (
              <p className="muted-text">Complete the rule to see how your resources score.</p>
            ) : preview.isError ? (
              <p className="pol-error">{(preview.error as Error).message}</p>
            ) : !e ? (
              <Loading label="Evaluating…" />
            ) : (
              <div className={preview.isFetching ? 'is-stale' : undefined}>
                <div className="pol-preview-score">{e.score == null ? '—' : `${e.score}%`}</div>
                <ScoreBar score={e.score} />
                <p className="muted-text">
                  {e.evaluated
                    ? `${(e.evaluated - e.noncompliant).toLocaleString()} of ${e.evaluated.toLocaleString()} resources comply`
                    : `No ${TYPE_LABELS[draft.resourceType]} resources match the scope`}
                </p>
                {preview.data!.violations.length > 0 && (
                  <>
                    <h4>Noncompliant resources</h4>
                    <ul className="pol-preview-list">
                      {preview.data!.violations.map((v) => (
                        <li key={v.resource.id}>
                          <span className="rc-name" title={v.resource.name}>
                            {v.resource.name}
                          </span>
                          <span className="muted-text">{v.reason}</span>
                        </li>
                      ))}
                    </ul>
                    {e.noncompliant > preview.data!.violations.length && <p className="muted-text">and {(e.noncompliant - preview.data!.violations.length).toLocaleString()} more</p>}
                  </>
                )}
              </div>
            )}
          </aside>
        </div>
      </div>
    </div>
  );
}
