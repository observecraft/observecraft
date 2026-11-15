import { useEffect, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ChevronLeft, Plus, Trash2 } from 'lucide-react';
import type { RecommendedView, TaggingPolicyInput, TagScope, TagTarget } from '../../api/tagging/types';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { snippetsFor } from './taggingSnippets';
import { TARGET_LABELS } from './TaggingPage';
import { useSaveTaggingPolicy, useTaggingPolicies, useTaggingPolicy } from './taggingQueries';
import './policies.css';
import './tagging.css';
import '../../components/provenance/provenance.css';

interface Draft {
  name: string;
  description: string;
  target: TagTarget;
  scope: Required<TagScope>;
  /** Allowed values as typed: comma-separated. */
  required: { key: string; allowed: string }[];
  enabled: boolean;
  recommended: string | null;
}

const EMPTY_SCOPE: Required<TagScope> = { provider: '', account: '', cluster: '', resourceType: '', names: '', excludeNamespaces: '', excludeLabels: '' };
const EMPTY: Draft = { name: '', description: '', target: 'cloud', scope: EMPTY_SCOPE, required: [{ key: '', allowed: '' }], enabled: true, recommended: null };

/** Scope fields per target: label, key and placeholder. */
const SCOPE_FIELDS: Record<TagTarget, [keyof TagScope, string, string][]> = {
  cloud: [
    ['provider', 'Cloud provider', 'aws, azure, gcp'],
    ['account', 'Account, subscription or project', '123456789012'],
    ['resourceType', 'Resource type', 'AwsS3Bucket, Microsoft.Storage/*'],
    ['names', 'Resource names', 'example-shop-*'],
  ],
  kubernetes: [
    ['cluster', 'Cluster', 'shop-prod-*'],
    ['resourceType', 'Kind', 'kube_deployment, kube_service'],
    ['names', 'Object names', 'storefront-*'],
    ['excludeNamespaces', 'Except namespaces', 'kube-system, gke-managed-*'],
    ['excludeLabels', 'Except objects labelled', 'addonmanager.kubernetes.io/mode, app:coredns'],
  ],
  image: [
    ['cluster', 'Cluster', 'shop-prod-*'],
    ['names', 'Image names', 'registry.example.com/*'],
  ],
  service: [['names', 'Services', 'storefront-api, order-*']],
};

const fromRecommended = (r: RecommendedView): Draft => ({
  name: r.name,
  description: r.description,
  target: r.rule.target,
  scope: { ...EMPTY_SCOPE, ...r.rule.scope },
  required: r.rule.required.map((t) => ({ key: t.key, allowed: (t.allowedValues ?? []).join(', ') })),
  enabled: true,
  recommended: r.key,
});

function toInput(d: Draft): TaggingPolicyInput {
  return {
    name: d.name.trim(),
    description: d.description.trim(),
    target: d.target,
    scope: d.scope,
    required: d.required
      .filter((t) => t.key.trim())
      .map((t) => ({
        key: t.key.trim(),
        allowedValues: t.allowed
          .split(',')
          .map((v) => v.trim())
          .filter(Boolean),
      })),
    enabled: d.enabled,
    recommended: d.recommended,
  };
}

/** What's wrong with the draft, or null. */
function problem(d: Draft): string | null {
  const keys = d.required.map((t) => t.key.trim()).filter(Boolean);
  if (!keys.length) return 'Add at least one required tag.';
  const bad = keys.find((k) => k.split('|').some((a) => !a.trim() || /[:\s]/.test(a.trim())));
  if (bad) return `'${bad}' is not a valid tag key (no spaces or colons; separate alternatives with |).`;
  return null;
}

/** Create or edit a tagging policy, from scratch or from a recommended one, with the snippets that add its tags. */
export function TaggingPolicyEditorPage() {
  const { id } = useParams();
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const list = useTaggingPolicies();
  const existing = useTaggingPolicy(id);
  const save = useSaveTaggingPolicy();
  const [draft, setDraft] = useState<Draft>(EMPTY);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    if (loaded) return;
    if (id && existing.data) {
      const p = existing.data;
      setDraft({
        name: p.name,
        description: p.description,
        target: p.target,
        scope: { ...EMPTY_SCOPE, ...p.scope },
        required: p.required.map((t) => ({ key: t.key, allowed: (t.allowedValues ?? []).join(', ') })),
        enabled: p.enabled,
        recommended: p.recommended ?? null,
      });
      setLoaded(true);
    } else if (!id && list.data) {
      const from = list.data.recommended.find((r) => r.key === params.get('from'));
      if (from) setDraft(fromRecommended(from));
      setLoaded(true);
    }
  }, [id, existing.data, list.data, params, loaded]);

  if (id && existing.isPending) return <Loading label="Loading policy…" />;
  if (id && existing.isError) return <ErrorState error={existing.error} onRetry={() => existing.refetch()} />;

  const set = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));
  const setReq = (i: number, patch: Partial<Draft['required'][number]>) => set({ required: draft.required.map((t, j) => (j === i ? { ...t, ...patch } : t)) });
  const issue = problem(draft);
  const canSave = !!draft.name.trim() && !issue && !save.isPending;
  const submit = () => save.mutate({ id, input: toInput(draft) }, { onSuccess: () => navigate('/infrastructure/tagging/policies') });

  return (
    <div className="rc-page">
      <div className="hosts-scroll pol-editor">
        <Link to="/infrastructure/tagging/policies" className="pol-back">
          <ChevronLeft size={14} /> Tagging policies
        </Link>
        <h1 className="pol-title">{id ? 'Edit Tagging Policy' : 'Create a Tagging Policy'}</h1>
        <div className="pol-editor-grid">
          <div className="pol-form">
            {!id && list.data && (
              <section className="pol-step">
                <h3>Start from a recommended policy</h3>
                <div className="tg-presets">
                  {list.data.recommended.map((r) => (
                    <button key={r.key} className={`btn${draft.recommended === r.key ? ' btn-primary' : ''}`} title={r.description} onClick={() => setDraft(fromRecommended(r))}>
                      {r.name}
                    </button>
                  ))}
                </div>
              </section>
            )}

            <section className="pol-step">
              <h3>
                <span className="pol-step-n">1</span> Choose what it applies to
              </h3>
              <Select
                label="Target"
                value={draft.target}
                options={(Object.keys(TARGET_LABELS) as TagTarget[]).map((t) => ({ value: t, label: TARGET_LABELS[t] }))}
                onChange={(t) => set({ target: t, scope: EMPTY_SCOPE })}
              />
            </section>

            <section className="pol-step">
              <h3>
                <span className="pol-step-n">2</span> Narrow the scope <span className="muted-text">(optional)</span>
              </h3>
              <div className="tg-scope">
                {SCOPE_FIELDS[draft.target].map(([key, label, placeholder]) => (
                  <label key={key} className="pol-field">
                    <span>{label}</span>
                    <input className="pol-input mono" value={draft.scope[key]} placeholder={placeholder} onChange={(e) => set({ scope: { ...draft.scope, [key]: e.target.value } })} />
                  </label>
                ))}
              </div>
              <p className="pol-hint">Comma-separated values with * wildcards; empty for all.</p>
            </section>

            <section className="pol-step">
              <h3>
                <span className="pol-step-n">3</span> Required tags
              </h3>
              <p className="pol-hint">
                Every resource in scope must carry each tag. Separate alternatives with | (tf_repo|git_repo accepts Yor’s tags); keys ignore case, - and _ alike. Allowed values are optional.
              </p>
              {draft.required.map((t, i) => (
                <div key={i} className="tg-required">
                  <input className="pol-input mono" placeholder="tag key (e.g. team)" value={t.key} onChange={(e) => setReq(i, { key: e.target.value })} />
                  <input className="pol-input mono" placeholder="allowed values (any when empty)" value={t.allowed} onChange={(e) => setReq(i, { allowed: e.target.value })} />
                  <button className="icon-btn" title="Remove tag" disabled={draft.required.length === 1} onClick={() => set({ required: draft.required.filter((_, j) => j !== i) })}>
                    <Trash2 size={14} />
                  </button>
                </div>
              ))}
              <button className="link-btn" disabled={draft.required.length >= 20} onClick={() => set({ required: [...draft.required, { key: '', allowed: '' }] })}>
                <Plus size={14} /> Add tag
              </button>
              {issue && draft.required.some((t) => t.key.trim()) && <p className="pol-error">{issue}</p>}
            </section>

            <section className="pol-step">
              <h3>
                <span className="pol-step-n">4</span> Name it
              </h3>
              <label className="pol-field">
                <span>Name</span>
                <input className="pol-input" value={draft.name} maxLength={200} placeholder="Buckets name a cost center" onChange={(e) => set({ name: e.target.value })} />
              </label>
              <label className="pol-field">
                <span>Description</span>
                <textarea
                  className="pol-input"
                  rows={3}
                  maxLength={2000}
                  value={draft.description}
                  placeholder="Why the tags matter and where to add them."
                  onChange={(e) => set({ description: e.target.value })}
                />
              </label>
              <label className="prov-toggle">
                <button type="button" role="switch" aria-checked={draft.enabled} className={`switch${draft.enabled ? ' on' : ''}`} onClick={() => set({ enabled: !draft.enabled })} />
                Enabled (evaluated, and reported as Cloud Security misconfigurations)
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

          <aside className="pol-preview tg-snippets">
            <h3>How to add these tags</h3>
            {snippetsFor(draft.target).map((s) => (
              <div key={s.key}>
                <h4>{s.title}</h4>
                <p className="muted">{s.summary}</p>
                <CodeBlock code={s.code} filename={s.language} />
              </div>
            ))}
          </aside>
        </div>
      </div>
    </div>
  );
}
