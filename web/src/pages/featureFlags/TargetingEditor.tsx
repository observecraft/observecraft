import { ArrowDown, ArrowUp, Pause, Plus, RotateCcw, Trash2, X } from 'lucide-react';
import type { FlagCondition, FlagEnvConfig, FlagRollout, FlagRule, FlagServe, FlagVariant } from '../../api/featureFlags/client';
import { OPERATORS, evenSplit, rolloutPercent, rolloutStepIndex, splitTotal, splitValid, variantColor } from '../../api/featureFlags/logic';
import { Swatch, Switch } from './common';

const DURATION_UNITS = [
  { label: 'minutes', minutes: 1 },
  { label: 'hours', minutes: 60 },
  { label: 'days', minutes: 1440 },
];

const unitOf = (minutes: number) => [...DURATION_UNITS].reverse().find((u) => minutes > 0 && minutes % u.minutes === 0) ?? DURATION_UNITS[0];

/** One environment's targeting: on/off, ordered rules, the default rule and a progressive rollout. */
export function TargetingEditor({ config, variants, onChange, envName }: { config: FlagEnvConfig; variants: FlagVariant[]; onChange: (c: FlagEnvConfig) => void; envName: string }) {
  const rules = config.rules ?? [];
  const set = (p: Partial<FlagEnvConfig>) => onChange({ ...config, ...p });
  const setRule = (i: number, r: FlagRule) => set({ rules: rules.map((x, j) => (j === i ? r : x)) });
  const move = (i: number, by: number) => {
    const next = [...rules];
    const [r] = next.splice(i, 1);
    next.splice(i + by, 0, r);
    set({ rules: next });
  };
  const addRule = () => set({ rules: [...rules, { name: '', conditions: [{ attribute: '', operator: 'one_of', values: [] }], serve: { variant: variants[0]?.key } }] });
  return (
    <div className="ff-targeting">
      <div className="ff-field">
        <Switch
          checked={!!config.enabled}
          onChange={(enabled) => set({ enabled })}
          label={config.enabled ? `On in ${envName}: the rules below decide what SDKs get` : `Off in ${envName}: SDKs fall back to the default value in their code`}
        />
      </div>
      <h4 className="ff-label">Targeting rules</h4>
      <p className="ff-hint">
        Rules are checked from top to bottom; the first one whose conditions all hold serves. Attributes come from the evaluation context (targetingKey, email, country, plan, user.tier …).
      </p>
      {rules.map((r, i) => (
        <RuleEditor
          key={i}
          n={i + 1}
          rule={r}
          variants={variants}
          onChange={(x) => setRule(i, x)}
          onRemove={() => set({ rules: rules.filter((_, j) => j !== i) })}
          onUp={i > 0 ? () => move(i, -1) : undefined}
          onDown={i < rules.length - 1 ? () => move(i, 1) : undefined}
        />
      ))}
      <button type="button" className="btn btn-sm" onClick={addRule} disabled={rules.length >= 50}>
        <Plus size={14} /> Add Rule
      </button>
      <h4 className="ff-label" style={{ marginTop: 18 }}>
        Default rule
      </h4>
      <div className="ff-default-rule">
        <ServeEditor label={rules.length ? 'Else' : 'Serve'} serve={config.defaultServe ?? {}} variants={variants} onChange={(defaultServe) => set({ defaultServe })} />
      </div>
      <h4 className="ff-label" style={{ marginTop: 18 }}>
        Progressive rollout
      </h4>
      <RolloutEditor rollout={config.rollout ?? null} variants={variants} onChange={(rollout) => set({ rollout })} />
    </div>
  );
}

function RuleEditor({
  n,
  rule,
  variants,
  onChange,
  onRemove,
  onUp,
  onDown,
}: {
  n: number;
  rule: FlagRule;
  variants: FlagVariant[];
  onChange: (r: FlagRule) => void;
  onRemove: () => void;
  onUp?: () => void;
  onDown?: () => void;
}) {
  const conditions = rule.conditions ?? [];
  const setCond = (i: number, c: FlagCondition) => onChange({ ...rule, conditions: conditions.map((x, j) => (j === i ? c : x)) });
  return (
    <div className="ff-rule">
      <span className="ff-rule-n">{n}</span>
      <div className="ff-rule-head">
        <input className="ff-input" aria-label={`Rule ${n} name`} placeholder="Rule name (optional)" value={rule.name ?? ''} onChange={(e) => onChange({ ...rule, name: e.target.value })} />
        <span className="ff-rule-tools">
          <button type="button" className="icon-plain" title="Move up" disabled={!onUp} onClick={onUp}>
            <ArrowUp size={15} />
          </button>
          <button type="button" className="icon-plain" title="Move down" disabled={!onDown} onClick={onDown}>
            <ArrowDown size={15} />
          </button>
          <button type="button" className="icon-plain" title="Remove rule" onClick={onRemove}>
            <Trash2 size={15} />
          </button>
        </span>
      </div>
      {conditions.map((c, i) => (
        <ConditionRow key={i} first={i === 0} cond={c} onChange={(x) => setCond(i, x)} onRemove={() => onChange({ ...rule, conditions: conditions.filter((_, j) => j !== i) })} />
      ))}
      <div className="ff-cond">
        <span />
        <button
          type="button"
          className="link-btn"
          style={{ justifySelf: 'start' }}
          onClick={() => onChange({ ...rule, conditions: [...conditions, { attribute: '', operator: 'one_of', values: [] }] })}
        >
          + Add condition
        </button>
      </div>
      {!conditions.length && <p className="ff-hint">Without conditions the rule matches every subject.</p>}
      <ServeEditor label="Serve" serve={rule.serve} variants={variants} onChange={(serve) => onChange({ ...rule, serve })} />
    </div>
  );
}

function ConditionRow({ first, cond, onChange, onRemove }: { first: boolean; cond: FlagCondition; onChange: (c: FlagCondition) => void; onRemove: () => void }) {
  const op = OPERATORS.find((o) => o.id === cond.operator) ?? OPERATORS[0];
  const values = cond.values ?? [];
  return (
    <div className="ff-cond">
      <span className="ff-if">{first ? 'If' : 'And'}</span>
      <input className="ff-input" aria-label="Attribute" placeholder="attribute, e.g. country" value={cond.attribute} onChange={(e) => onChange({ ...cond, attribute: e.target.value })} />
      <select aria-label="Operator" value={cond.operator} onChange={(e) => onChange({ ...cond, operator: e.target.value as FlagCondition['operator'] })}>
        {OPERATORS.map((o) => (
          <option key={o.id} value={o.id}>
            {o.label}
          </option>
        ))}
      </select>
      {op.values === 'none' ? (
        <span />
      ) : (
        <input
          className="ff-input"
          aria-label="Values"
          placeholder={op.values === 'one' ? 'a number' : 'values, comma separated'}
          value={values.join(', ')}
          onChange={(e) =>
            onChange({
              ...cond,
              values: op.values === 'one' ? [e.target.value.trim()] : e.target.value.split(',').map((v) => v.trim()),
            })
          }
          onBlur={() => onChange({ ...cond, values: values.filter((v) => v !== '') })}
        />
      )}
      <button type="button" className="icon-plain" title="Remove condition" onClick={onRemove}>
        <X size={15} />
      </button>
    </div>
  );
}

/** A variant, or a percentage split between variants. */
export function ServeEditor({ label, serve, variants, onChange }: { label: string; serve: FlagServe; variants: FlagVariant[]; onChange: (s: FlagServe) => void }) {
  const split = serve.split ?? [];
  const isSplit = !serve.variant && split.length > 0;
  const keys = variants.map((v) => v.key);
  const colorOf = (k: string) => variantColor(Math.max(0, keys.indexOf(k)));
  const choose = (v: string) => onChange(v === '__split' ? { split: evenSplit(keys) } : { variant: v });
  const total = splitTotal(split);
  return (
    <div className="ff-serve">
      <span className="ff-if">{label}</span>
      <select aria-label={`${label}: variant`} value={isSplit ? '__split' : (serve.variant ?? '')} onChange={(e) => choose(e.target.value)}>
        {!serve.variant && !isSplit && <option value="">Choose…</option>}
        {variants.map((v) => (
          <option key={v.key} value={v.key}>
            {v.key}
          </option>
        ))}
        <option value="__split">Percentage split…</option>
      </select>
      {isSplit && (
        <div className="ff-split">
          <div className="ff-split-bar" aria-hidden>
            {split.map((p) => (
              <span key={p.variant} style={{ width: `${Math.max(0, p.weight)}%`, background: colorOf(p.variant) }} />
            ))}
          </div>
          {variants.map((v) => {
            const part = split.find((p) => p.variant === v.key);
            return (
              <div key={v.key} className="ff-split-row">
                <Swatch color={colorOf(v.key)} />
                <span>{v.key}</span>
                <input
                  className="ff-input"
                  type="number"
                  min={0}
                  max={100}
                  step={0.01}
                  aria-label={`${v.key} percent`}
                  value={part?.weight ?? 0}
                  onChange={(e) => {
                    const weight = Math.round(Number(e.target.value) * 100) / 100;
                    const others = split.filter((p) => p.variant !== v.key);
                    const next = weight > 0 ? [...others, { variant: v.key, weight }] : others;
                    onChange({ split: keys.flatMap((k) => next.filter((p) => p.variant === k)) });
                  }}
                />
                <span className="muted">%</span>
              </div>
            );
          })}
          <span className={`ff-split-total${splitValid(split) ? '' : ' bad'}`}>
            Total {total}%{splitValid(split) ? '' : ' — the split must add up to 100%'}. Each targetingKey keeps its variant.
          </span>
        </div>
      )}
    </div>
  );
}

function RolloutEditor({ rollout, variants, onChange }: { rollout: FlagRollout | null; variants: FlagVariant[]; onChange: (r: FlagRollout | undefined) => void }) {
  if (!rollout) {
    return (
      <div className="ff-rollout">
        <p className="ff-hint">Move the traffic that no rule matches to one variant in steps (5% for an hour, then 25%, …). Subjects already moved stay moved as the percentage grows.</p>
        <div>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() =>
              onChange({
                variant: variants[0]?.key ?? '',
                steps: [
                  { percent: 5, durationMinutes: 60 },
                  { percent: 25, durationMinutes: 240 },
                  { percent: 50, durationMinutes: 1440 },
                  { percent: 100, durationMinutes: 0 },
                ],
                startedAt: 0,
              })
            }
          >
            <Plus size={14} /> Set up a rollout
          </button>
        </div>
      </div>
    );
  }
  const now = Date.now();
  const running = (rollout.startedAt ?? 0) > 0;
  const current = rolloutStepIndex(rollout, now);
  const percent = rolloutPercent(rollout, now);
  const setStep = (i: number, p: Partial<FlagRollout['steps'][number]>) => onChange({ ...rollout, steps: rollout.steps.map((s, j) => (j === i ? { ...s, ...p } : s)) });
  return (
    <div className="ff-rollout">
      <div className="ff-serve">
        <span className="ff-if">Roll out</span>
        <select aria-label="Rollout variant" value={rollout.variant} onChange={(e) => onChange({ ...rollout, variant: e.target.value })}>
          {variants.map((v) => (
            <option key={v.key} value={v.key}>
              {v.key}
            </option>
          ))}
        </select>
        <span className="muted">to the traffic the rules don’t match</span>
      </div>
      {running ? (
        <div>
          <div className="ff-progress" aria-label={`Rollout at ${percent}%`}>
            <span style={{ width: `${percent}%` }} />
          </div>
          <p className="ff-hint">
            Running since {new Date(rollout.startedAt!).toLocaleString()}: step {current + 1} of {rollout.steps.length}, {percent}% get {rollout.variant}.
          </p>
        </div>
      ) : (
        <p className="ff-hint">Starts when you save.</p>
      )}
      <div className="ff-rollout-steps">
        {rollout.steps.map((s, i) => {
          const unit = unitOf(s.durationMinutes);
          const last = i === rollout.steps.length - 1;
          return (
            <div key={i} className={`ff-rollout-step${i === current ? ' current' : ''}`}>
              <span>Step {i + 1}</span>
              <input className="ff-input" type="number" min={0} max={100} aria-label={`Step ${i + 1} percent`} value={s.percent} onChange={(e) => setStep(i, { percent: Number(e.target.value) })} />
              <span className="muted">%</span>
              {last ? (
                <span className="muted">until changed</span>
              ) : (
                <span style={{ display: 'flex', gap: 6 }}>
                  <input
                    className="ff-input"
                    type="number"
                    min={1}
                    style={{ width: 60 }}
                    aria-label={`Step ${i + 1} duration`}
                    value={Math.round(s.durationMinutes / unit.minutes)}
                    onChange={(e) => setStep(i, { durationMinutes: Math.max(1, Math.round(Number(e.target.value))) * unit.minutes })}
                  />
                  <select
                    aria-label={`Step ${i + 1} unit`}
                    value={unit.label}
                    onChange={(e) => setStep(i, { durationMinutes: Math.max(1, Math.round(s.durationMinutes / unit.minutes)) * DURATION_UNITS.find((u) => u.label === e.target.value)!.minutes })}
                  >
                    {DURATION_UNITS.map((u) => (
                      <option key={u.label}>{u.label}</option>
                    ))}
                  </select>
                </span>
              )}
              <button
                type="button"
                className="icon-plain"
                title="Remove step"
                disabled={rollout.steps.length <= 1}
                onClick={() => onChange({ ...rollout, steps: rollout.steps.filter((_, j) => j !== i) })}
              >
                <X size={15} />
              </button>
            </div>
          );
        })}
      </div>
      <div style={{ display: 'flex', gap: 8 }}>
        <button
          type="button"
          className="btn btn-sm"
          disabled={rollout.steps.length >= 20}
          onClick={() => {
            const steps = rollout.steps.map((s, i) => (i === rollout.steps.length - 1 && s.durationMinutes === 0 ? { ...s, durationMinutes: 60 } : s));
            onChange({ ...rollout, steps: [...steps, { percent: 100, durationMinutes: 0 }] });
          }}
        >
          <Plus size={14} /> Add Step
        </button>
        {running && (
          <button type="button" className="btn btn-sm" onClick={() => onChange({ ...rollout, startedAt: 0 })}>
            <RotateCcw size={14} /> Restart
          </button>
        )}
        <button type="button" className="btn btn-sm" onClick={() => onChange(undefined)}>
          <Pause size={14} /> {running ? 'Stop rollout' : 'Remove'}
        </button>
      </div>
    </div>
  );
}
