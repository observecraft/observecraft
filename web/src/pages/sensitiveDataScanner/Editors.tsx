// The group and rule editors of Organization Settings → Sensitive Data Scanner, and the rule tester.
import { useEffect, useMemo, useState, type FormEvent } from 'react';
import { X } from 'lucide-react';
import {
  CATEGORY_LABELS,
  REPLACEMENT_KINDS,
  REPLACEMENT_LABELS,
  type ReplacementKind,
  type SdsGroup,
  type SdsGroupInput,
  type SdsRule,
  type SdsRuleInput,
  type SdsStandardPattern,
  type SdsTestRequest,
} from '../../api/sensitiveDataScanner/client';
import { Modal } from '../../components/ui/Modal';
import { groupInput, ruleInput, useRuleTest, useSaveGroup, useSaveRule } from './queries';

const PRODUCTS: [string, string][] = [
  ['logs', 'Logs'],
  ['rum', 'RUM'],
  ['events', 'Events'],
  ['apm', 'APM'],
];

/** Comma- or line-separated values. */
const list = (s: string) =>
  s
    .split(/[,\n]/)
    .map((v) => v.trim())
    .filter(Boolean);

/** `value`, once it has stopped changing for `ms`. */
function useSettled<T>(value: T, ms = 400): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return settled;
}

function Title({ text, onClose }: { text: string; onClose: () => void }) {
  return (
    <div className="modal-title-row">
      <h3>{text}</h3>
      <button type="button" className="icon-plain" onClick={onClose} title="Close">
        <X size={16} />
      </button>
    </div>
  );
}

export function GroupEditor({ group, onClose }: { group: SdsGroup | null; onClose: () => void }) {
  const [form, setForm] = useState<SdsGroupInput>(group ? groupInput(group) : { name: '', description: '', query: '', enabled: true, products: ['logs'], samplingRate: 100 });
  const save = useSaveGroup();
  const set = (patch: Partial<SdsGroupInput>) => setForm((f) => ({ ...f, ...patch }));
  const products = form.products ?? [];
  const submit = (e: FormEvent) => {
    e.preventDefault();
    save.mutate({ id: group?.id, input: form }, { onSuccess: onClose });
  };
  return (
    <Modal onClose={onClose} label={group ? 'Edit scanning group' : 'New scanning group'} width={640} height="auto">
      <form className="small-modal sds-form" onSubmit={submit}>
        <Title text={group ? 'Edit scanning group' : 'New scanning group'} onClose={onClose} />
        <label className="form-label">
          Name
          <input value={form.name} onChange={(e) => set({ name: e.target.value })} placeholder="PCI compliance" required autoFocus />
        </label>
        <label className="form-label">
          Description
          <input value={form.description ?? ''} onChange={(e) => set({ description: e.target.value })} placeholder="What this group protects" />
        </label>
        <label className="form-label">
          Logs to scan
          <input className="mono" value={form.query ?? ''} onChange={(e) => set({ query: e.target.value })} placeholder="service:billing env:prod (empty: every log)" />
          <span className="muted sds-hint">Log search syntax. The group's rules run only on the logs it matches.</span>
        </label>
        <div className="form-label">
          Products
          <div className="sds-checks">
            {PRODUCTS.map(([id, label]) => (
              <label key={id} className="sds-check">
                <input type="checkbox" checked={products.includes(id)} onChange={(e) => set({ products: e.target.checked ? [...products, id] : products.filter((p) => p !== id) })} />
                {label}
              </label>
            ))}
          </div>
          <span className="muted sds-hint">ObserveCraft scans logs; the other products are kept for Terraform and API clients.</span>
        </div>
        <label className="form-label sds-narrow">
          Sampling rate (%)
          <input type="number" min={0} max={100} step="any" value={form.samplingRate ?? 100} onChange={(e) => set({ samplingRate: e.target.value === '' ? null : Number(e.target.value) })} />
          <span className="muted sds-hint">Share of the matching logs scanned. Lower it to cut the cost of scanning busy services.</span>
        </label>
        <label className="sds-check">
          <input type="checkbox" checked={form.enabled ?? false} onChange={(e) => set({ enabled: e.target.checked })} /> Enabled
        </label>
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {group ? 'Save' : 'Create group'}
          </button>
        </div>
      </form>
    </Modal>
  );
}

const EMPTY_RULE = (groupId: string, standardPatternId?: string): SdsRuleInput => ({
  groupId,
  name: '',
  description: '',
  pattern: '',
  standardPatternId: standardPatternId ?? null,
  namespaces: [],
  excludedNamespaces: [],
  tags: [],
  enabled: true,
  priority: null,
  replacement: { kind: 'replacement_string', replacementString: '[redacted]' },
  keywords: [],
  keywordCharacters: null,
  useRecommendedKeywords: false,
  suppressions: { startsWith: [], endsWith: [], exactMatch: [] },
});

/** Text fields kept as typed (lists are split on save). */
interface Texts {
  tags: string;
  namespaces: string;
  excluded: string;
  keywords: string;
  startsWith: string;
  endsWith: string;
  exactMatch: string;
}

const textsOf = (r: SdsRuleInput): Texts => ({
  tags: (r.tags ?? []).join(', '),
  namespaces: (r.namespaces ?? []).join(', '),
  excluded: (r.excludedNamespaces ?? []).join(', '),
  keywords: (r.keywords ?? []).join(', '),
  startsWith: (r.suppressions?.startsWith ?? []).join(', '),
  endsWith: (r.suppressions?.endsWith ?? []).join(', '),
  exactMatch: (r.suppressions?.exactMatch ?? []).join(', '),
});

const SAMPLE = 'checkout failed for alice@example.com, card 4111 1111 1111 1111, key AKIAIOSFODNN7EXAMPLE';

/** A new rule (`rule` null) of `groupId`, or an existing one. `groups` lets a new rule pick its group. */
export function RuleEditor({
  rule,
  groupId,
  groups,
  patterns,
  standardPatternId,
  onClose,
  onSaved,
}: {
  rule: SdsRule | null;
  groupId: string;
  groups: SdsGroup[];
  patterns: SdsStandardPattern[];
  standardPatternId?: string;
  onClose: () => void;
  /** After a successful save (before closing). */
  onSaved?: (rule: SdsRule) => void;
}) {
  const initial = rule ? ruleInput(rule) : EMPTY_RULE(groupId, standardPatternId);
  const [form, setForm] = useState<SdsRuleInput>(initial);
  const [texts, setTexts] = useState<Texts>(textsOf(initial));
  const [source, setSource] = useState<'library' | 'custom'>(initial.standardPatternId ? 'library' : 'custom');
  const [sample, setSample] = useState(SAMPLE);
  const save = useSaveRule();
  const set = (patch: Partial<SdsRuleInput>) => setForm((f) => ({ ...f, ...patch }));
  const text = (patch: Partial<Texts>) => setTexts((t) => ({ ...t, ...patch }));
  const std = patterns.find((p) => p.id === form.standardPatternId);
  const kind = (form.replacement?.kind ?? 'none') as ReplacementKind;

  /** The rule as sent: lists split, the pattern source applied. */
  const built = useMemo<SdsRuleInput>(
    () => ({
      ...form,
      name: form.name || 'Untitled rule',
      pattern: source === 'custom' ? form.pattern : null,
      standardPatternId: source === 'library' ? form.standardPatternId : null,
      useRecommendedKeywords: source === 'library' && form.useRecommendedKeywords,
      tags: list(texts.tags),
      namespaces: list(texts.namespaces),
      excludedNamespaces: list(texts.excluded),
      keywords: list(texts.keywords),
      suppressions: { startsWith: list(texts.startsWith), endsWith: list(texts.endsWith), exactMatch: list(texts.exactMatch) },
    }),
    [form, texts, source],
  );
  // Settled as text: a new object every render would never settle.
  const ready = source === 'custom' ? !!form.pattern : !!form.standardPatternId;
  const settled = useSettled(ready ? JSON.stringify({ rule: built, sample }) : '');
  const testReq = useMemo(() => (settled ? (JSON.parse(settled) as SdsTestRequest) : null), [settled]);
  const test = useRuleTest(testReq);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    save.mutate({ id: rule?.id, input: { ...built, name: form.name } }, { onSuccess: (saved) => (onSaved?.(saved), onClose()) });
  };
  const byCategory = Object.entries(CATEGORY_LABELS).map(([id, label]) => [label, patterns.filter((p) => p.category === id)] as const);
  return (
    <Modal onClose={onClose} label={rule ? 'Edit scanning rule' : 'New scanning rule'} width={860} height="min(940px, 100%)">
      <form className="small-modal sds-form sds-rule-form" onSubmit={submit}>
        <Title text={rule ? 'Edit scanning rule' : 'New scanning rule'} onClose={onClose} />
        <div className="sds-grid">
          <label className="form-label">
            Name
            <input value={form.name} onChange={(e) => set({ name: e.target.value })} placeholder="Card numbers" required autoFocus />
          </label>
          {rule ? (
            <label className="form-label">
              Group
              <input value={groups.find((g) => g.id === form.groupId)?.name ?? ''} disabled />
            </label>
          ) : (
            <label className="form-label">
              Group
              <select value={form.groupId} onChange={(e) => set({ groupId: e.target.value })} required>
                {groups.map((g) => (
                  <option key={g.id} value={g.id}>
                    {g.name}
                  </option>
                ))}
              </select>
            </label>
          )}
        </div>
        <label className="form-label">
          Description
          <input value={form.description ?? ''} onChange={(e) => set({ description: e.target.value })} />
        </label>

        <fieldset className="sds-fieldset">
          <legend>What to find</legend>
          <div className="sds-checks">
            <label className="sds-check">
              <input type="radio" checked={source === 'library'} onChange={() => setSource('library')} /> A pattern of the library
            </label>
            <label className="sds-check">
              <input type="radio" checked={source === 'custom'} onChange={() => setSource('custom')} /> My own regular expression
            </label>
          </div>
          {source === 'library' ? (
            <>
              <select value={form.standardPatternId ?? ''} onChange={(e) => set({ standardPatternId: e.target.value || null })} required aria-label="Standard pattern">
                <option value="">Choose a pattern…</option>
                {byCategory.map(([label, items]) => (
                  <optgroup key={label} label={label}>
                    {items.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                  </optgroup>
                ))}
              </select>
              {std && <p className="muted sds-hint">{std.description}</p>}
            </>
          ) : (
            <>
              <input
                className="mono"
                value={form.pattern ?? ''}
                onChange={(e) => set({ pattern: e.target.value })}
                placeholder={String.raw`sess_[A-Za-z0-9]{24}`}
                aria-label="Regular expression"
                required
              />
              <span className="muted sds-hint">Rust regular expression syntax: no lookaround or backreferences, and matching always runs in linear time.</span>
            </>
          )}
        </fieldset>

        <fieldset className="sds-fieldset">
          <legend>What to do with a match</legend>
          <div className="sds-grid">
            <label className="form-label">
              Action
              <select value={kind} onChange={(e) => set({ replacement: { ...form.replacement, kind: e.target.value } })}>
                {REPLACEMENT_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {REPLACEMENT_LABELS[k]}
                  </option>
                ))}
              </select>
            </label>
            {kind === 'replacement_string' && (
              <label className="form-label">
                Replacement
                <input value={form.replacement?.replacementString ?? ''} onChange={(e) => set({ replacement: { ...form.replacement!, replacementString: e.target.value } })} required />
              </label>
            )}
            {kind.startsWith('partial_') && (
              <label className="form-label">
                Characters masked
                <input
                  type="number"
                  min={0}
                  value={form.replacement?.numberOfChars ?? ''}
                  onChange={(e) => set({ replacement: { ...form.replacement!, numberOfChars: e.target.value === '' ? null : Number(e.target.value) } })}
                  required
                />
              </label>
            )}
          </div>
          <span className="muted sds-hint">
            {kind === 'none'
              ? 'The value stays; matching logs get the tags below.'
              : kind === 'hash'
                ? 'The value becomes a 16-character hash, the same for the same value, so you can still group by it.'
                : 'Matching logs also get the tags below.'}
          </span>
          <div className="sds-grid">
            <label className="form-label">
              Tags
              <input value={texts.tags} onChange={(e) => text({ tags: e.target.value })} placeholder={std ? std.tags.join(', ') : 'sensitive_data:session_token'} />
            </label>
            <label className="form-label">
              Priority
              <select value={form.priority ?? ''} onChange={(e) => set({ priority: e.target.value ? Number(e.target.value) : null })}>
                <option value="">{std ? `Pattern's (${std.priority})` : 'Default (3)'}</option>
                {[1, 2, 3, 4, 5].map((n) => (
                  <option key={n} value={n}>
                    {n}
                    {n === 1 ? ' (highest)' : n === 5 ? ' (lowest)' : ''}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </fieldset>

        <fieldset className="sds-fieldset">
          <legend>Where to look</legend>
          <div className="sds-grid">
            <label className="form-label">
              Only these attributes
              <input className="mono" value={texts.namespaces} onChange={(e) => text({ namespaces: e.target.value })} placeholder="message, payment.card (empty: the whole log)" />
            </label>
            <label className="form-label">
              Except these attributes
              <input className="mono" value={texts.excluded} onChange={(e) => text({ excluded: e.target.value })} placeholder="payment.card_last4" />
            </label>
            <label className="form-label">
              Keywords near the match
              <input value={texts.keywords} onChange={(e) => text({ keywords: e.target.value })} placeholder="card, payment" />
            </label>
            <label className="form-label">
              Within (characters before)
              <input
                type="number"
                min={1}
                max={50}
                value={form.keywordCharacters ?? ''}
                placeholder="30"
                onChange={(e) => set({ keywordCharacters: e.target.value === '' ? null : Number(e.target.value) })}
              />
            </label>
          </div>
          {source === 'library' && (
            <label className="sds-check">
              <input type="checkbox" checked={form.useRecommendedKeywords ?? false} onChange={(e) => set({ useRecommendedKeywords: e.target.checked })} /> Also accept the pattern's recommended
              keywords
              {std && <span className="muted"> ({std.keywords.join(', ')})</span>}
            </label>
          )}
          <span className="muted sds-hint">With keywords, a match counts only when one of them appears shortly before it, or in the attribute's name.</span>
        </fieldset>

        <fieldset className="sds-fieldset">
          <legend>Ignore these values</legend>
          <div className="sds-grid three">
            <label className="form-label">
              Starting with
              <input value={texts.startsWith} onChange={(e) => text({ startsWith: e.target.value })} />
            </label>
            <label className="form-label">
              Ending with
              <input value={texts.endsWith} onChange={(e) => text({ endsWith: e.target.value })} placeholder="@example.com" />
            </label>
            <label className="form-label">
              Exactly
              <input value={texts.exactMatch} onChange={(e) => text({ exactMatch: e.target.value })} />
            </label>
          </div>
        </fieldset>

        <fieldset className="sds-fieldset">
          <legend>Test the rule</legend>
          <textarea className="mono sds-sample" rows={3} value={sample} onChange={(e) => setSample(e.target.value)} aria-label="Sample text" />
          {!testReq ? (
            <span className="muted">Choose a pattern to try it.</span>
          ) : test.isPending ? (
            <span className="muted">Scanning…</span>
          ) : test.isError ? (
            <span className="form-error">{(test.error as Error).message}</span>
          ) : (
            test.data && (
              <div className="sds-result">
                <span className={test.data.matches ? 'sds-badge hit' : 'sds-badge'}>
                  {test.data.matches} {test.data.matches === 1 ? 'match' : 'matches'}
                </span>
                <code className="sds-redacted">{test.data.redacted}</code>
                {test.data.tags.map((t) => (
                  <span key={t} className="role-chip">
                    {t}
                  </span>
                ))}
              </div>
            )
          )}
        </fieldset>

        <label className="sds-check">
          <input type="checkbox" checked={form.enabled ?? false} onChange={(e) => set({ enabled: e.target.checked })} /> Enabled
        </label>
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending || groups.length === 0}>
            {rule ? 'Save' : 'Create rule'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
