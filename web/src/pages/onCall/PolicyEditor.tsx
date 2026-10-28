import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { ArrowDown, CalendarDays, ChevronRight, Plus, Trash2, User, Users, X } from 'lucide-react';
import type { OnCallPolicyInput, OnCallStep, OnCallTargetKind } from '../../api/onCall/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { useUsers } from '../settings/queries';
import { errorText } from './common';
import { useDeleteOnCallPolicy, useOnCallPolicy, useOnCallSchedules, useOnCallTeams, useSaveOnCallPolicy } from './queries';
import '../monitors/monitors.css';
import './onCall.css';

const KIND_ICON = { user: User, schedule: CalendarDays, team: Users } as const;
const RESOLVE_OPTIONS = [
  { value: '0', label: 'Never' },
  { value: '30', label: '30 minutes' },
  { value: '60', label: '1 hour' },
  { value: '240', label: '4 hours' },
  { value: '720', label: '12 hours' },
  { value: '1440', label: '1 day' },
  { value: '10080', label: '1 week' },
];

/** On-Call → Escalation Policies → New / a policy. */
export function PolicyEditor() {
  const { id } = useParams();
  const q = useOnCallPolicy(id);
  if (id && q.isPending) return <Loading label="Loading escalation policy…" />;
  if (id && q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const initial: OnCallPolicyInput = q.data
    ? { name: q.data.name, team: q.data.team, tags: q.data.tags, steps: q.data.steps, repeat: q.data.repeat, resolveAfterMinutes: q.data.resolveAfterMinutes }
    : { name: '', team: null, tags: [], steps: [{ targets: [], escalateAfterMinutes: 5 }], repeat: 0, resolveAfterMinutes: null };
  return <Editor key={id ?? 'new'} id={id} initial={initial} usedBy={q.data?.usedBy ?? []} />;
}

function Editor({ id, initial, usedBy }: { id?: string; initial: OnCallPolicyInput; usedBy: string[] }) {
  const navigate = useNavigate();
  const [p, setP] = useState(initial);
  const [tagText, setTagText] = useState((initial.tags ?? []).join(', '));
  const save = useSaveOnCallPolicy(id);
  const remove = useDeleteOnCallPolicy();
  const users = useUsers({ offset: 0, limit: 200, statuses: ['active'] });
  const schedules = useOnCallSchedules();
  const teams = useOnCallTeams();
  const options: Record<OnCallTargetKind, { value: string; label: string }[]> = {
    user: (users.data?.users ?? []).map((u) => ({ value: u.id, label: u.name })),
    schedule: (schedules.data ?? []).map((s) => ({ value: s.id, label: s.name })),
    team: (teams.data ?? []).map((t) => ({ value: t.handle, label: t.name })),
  };
  const nameOf = (kind: OnCallTargetKind, id: string) => options[kind].find((o) => o.value === id)?.label ?? id;
  const setStep = (i: number, s: Partial<OnCallStep>) => setP({ ...p, steps: p.steps.map((x, j) => (j === i ? { ...x, ...s } : x)) });
  const ok = p.name.trim() && p.steps.length > 0 && p.steps.every((s) => s.targets.length > 0 && s.escalateAfterMinutes >= 1);
  const submit = () =>
    save.mutate(
      {
        ...p,
        tags: tagText
          .split(',')
          .map((t) => t.trim())
          .filter(Boolean),
      },
      { onSuccess: () => navigate('/on-call/escalation-policies') },
    );
  return (
    <div className="mon-layout onc-editor">
      <header className="product-header onc-editor-head">
        <div className="product-title onc-crumbs">
          <Link to="/on-call">On-Call</Link>
          <ChevronRight size={14} />
          <Link to="/on-call/escalation-policies">Escalation Policies</Link>
          <ChevronRight size={14} />
          <span>{id ? p.name || 'Escalation policy' : 'New Escalation Policy'}</span>
        </div>
        <div className="mon-header-actions">
          {id && (
            <button
              className="btn btn-danger"
              disabled={remove.isPending}
              onClick={() =>
                confirm(`Delete ${p.name}?${usedBy.length ? ` ${usedBy.map((t) => `@${t}`).join(', ')} will page every member instead.` : ''}`) &&
                remove.mutate(id, { onSuccess: () => navigate('/on-call/escalation-policies') })
              }
            >
              <Trash2 size={14} /> Delete
            </button>
          )}
          <button className="btn" onClick={() => navigate('/on-call/escalation-policies')}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!ok || save.isPending} onClick={submit}>
            {id ? 'Save' : 'Create'}
          </button>
        </div>
      </header>
      <div className="mon-section">
        <div className="onc-policy-form">
          {(save.error || remove.error) && <p className="form-error">{errorText(save.error ?? remove.error)}</p>}
          <section className="onc-card">
            <h2>Details</h2>
            <div className="onc-form-grid">
              <label className="form-label">
                Name
                <input value={p.name} maxLength={200} onChange={(e) => setP({ ...p, name: e.target.value })} placeholder="Checkout escalation" />
              </label>
              <Select label="Owned by" value={p.team ?? ''} onChange={(t) => setP({ ...p, team: t || null })} options={[{ value: '', label: 'No team' }, ...options.team]} />
              <label className="form-label">
                Tags
                <input value={tagText} onChange={(e) => setTagText(e.target.value)} placeholder="team:checkout, env:prod" />
              </label>
            </div>
          </section>
          <section className="onc-card">
            <h2>Escalation steps</h2>
            <p className="muted">When a page is triggered, the first step is notified. If nobody acknowledges in time, the next step is notified, and so on.</p>
            {p.steps.map((s, i) => (
              <div key={i}>
                <div className="onc-policy-step">
                  <div className="onc-policy-step-head">
                    <span className="onc-step-num">{i + 1}</span>
                    <b>Notify</b>
                    {p.steps.length > 1 && (
                      <button className="icon-btn onc-step-remove" title="Remove step" onClick={() => setP({ ...p, steps: p.steps.filter((_, j) => j !== i) })}>
                        <Trash2 size={13} />
                      </button>
                    )}
                  </div>
                  <div className="onc-targets">
                    {s.targets.map((t, ti) => {
                      const Icon = KIND_ICON[t.kind];
                      return (
                        <span key={`${t.kind}-${t.id}`} className="onc-target">
                          <Icon size={13} />
                          {nameOf(t.kind, t.id)}
                          <button className="icon-plain" title="Remove" onClick={() => setStep(i, { targets: s.targets.filter((_, j) => j !== ti) })}>
                            <X size={12} />
                          </button>
                        </span>
                      );
                    })}
                    <AddTarget options={options} onAdd={(kind, value) => !s.targets.some((t) => t.kind === kind && t.id === value) && setStep(i, { targets: [...s.targets, { kind, id: value }] })} />
                  </div>
                  <div className="onc-inline">
                    <span>If not acknowledged, escalate after</span>
                    <input type="number" className="onc-num" min={1} max={1440} value={s.escalateAfterMinutes} onChange={(e) => setStep(i, { escalateAfterMinutes: Number(e.target.value) })} />
                    <span>minutes</span>
                  </div>
                </div>
                {i < p.steps.length - 1 && (
                  <div className="onc-step-arrow">
                    <ArrowDown size={16} />
                  </div>
                )}
              </div>
            ))}
            <button className="btn onc-add-step" disabled={p.steps.length >= 10} onClick={() => setP({ ...p, steps: [...p.steps, { targets: [], escalateAfterMinutes: 10 }] })}>
              <Plus size={14} /> Add Step
            </button>
          </section>
          <section className="onc-card">
            <h2>When nobody responds</h2>
            <div className="onc-inline">
              <span>Repeat the policy</span>
              <input type="number" className="onc-num" min={0} max={10} value={p.repeat ?? 0} onChange={(e) => setP({ ...p, repeat: Number(e.target.value) })} />
              <span>more times after the last step</span>
            </div>
            <div className="onc-inline">
              <span>Resolve pages automatically after</span>
              <Select
                value={String(p.resolveAfterMinutes ?? 0)}
                onChange={(v) => setP({ ...p, resolveAfterMinutes: v === '0' ? null : Number(v) })}
                options={
                  RESOLVE_OPTIONS.some((o) => o.value === String(p.resolveAfterMinutes ?? 0))
                    ? RESOLVE_OPTIONS
                    : [...RESOLVE_OPTIONS, { value: String(p.resolveAfterMinutes), label: `${p.resolveAfterMinutes} minutes` }]
                }
              />
            </div>
            <p className="muted onc-hint">Notifications go out by email, Slack or webhook, following each responder's On-Call profile. There is no phone or SMS delivery.</p>
          </section>
        </div>
      </div>
    </div>
  );
}

function AddTarget({ options, onAdd }: { options: Record<OnCallTargetKind, { value: string; label: string }[]>; onAdd: (kind: OnCallTargetKind, value: string) => void }) {
  const [kind, setKind] = useState<OnCallTargetKind>('schedule');
  return (
    <span className="onc-add-target">
      <Select
        value={kind}
        onChange={setKind}
        options={[
          { value: 'schedule', label: 'Schedule' },
          { value: 'user', label: 'User' },
          { value: 'team', label: 'Team' },
        ]}
      />
      <Select value="" onChange={(v) => v && onAdd(kind, v)} options={[{ value: '', label: `Add a ${kind}…` }, ...options[kind]]} />
    </span>
  );
}
