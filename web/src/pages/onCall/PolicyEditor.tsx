import { useState } from 'react';
import { CalendarDays, CheckCircle2, CircleAlert, Clock, Plus, Repeat, Trash2, User, Users, X } from 'lucide-react';
import type { OnCallPolicyInput, OnCallStep, OnCallTargetKind } from '../../api/onCall/client';
import { minuteChoices, policyProblem } from '../../api/onCall/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { useUsers } from '../settings/queries';
import { errorText } from './common';
import { useDeleteOnCallPolicy, useOnCallPolicy, useOnCallSchedules, useOnCallTeams, useSaveOnCallPolicy, useSetOnCallRouting } from './queries';

const KIND_ICON = { user: User, schedule: CalendarDays, team: Users } as const;
const KIND_LABEL = { user: 'User', schedule: 'Schedule', team: 'Team' } as const;
const ESCALATE_MINUTES = [1, 5, 10, 15, 30, 60, 120, 240, 480, 1440];
const RESOLVE_MINUTES = [30, 60, 240, 720, 1440, 10080];
const MAX_STEPS = 10;

/** On-Call → Escalation Policies → New / a policy, as a dialog over the list. `team`: route that team to the new policy. */
export function PolicyModal({ id, team, onClose }: { id?: string; team?: string; onClose: () => void }) {
  const q = useOnCallPolicy(id);
  const body =
    id && q.isPending ? (
      <Loading label="Loading escalation policy…" />
    ) : id && q.isError ? (
      <ErrorState error={q.error} onRetry={() => q.refetch()} />
    ) : (
      <Editor
        key={id ?? 'new'}
        id={id}
        routeTeam={id ? undefined : team}
        initial={
          q.data
            ? { name: q.data.name, team: q.data.team, tags: q.data.tags, steps: q.data.steps, repeat: q.data.repeat, resolveAfterMinutes: q.data.resolveAfterMinutes }
            : { name: '', team: team ?? null, tags: [], steps: [{ targets: [], escalateAfterMinutes: 5 }], repeat: 0, resolveAfterMinutes: null }
        }
        usedBy={q.data?.usedBy ?? []}
        onClose={onClose}
      />
    );
  return (
    <Modal onClose={onClose} label={id ? 'Escalation Policy' : 'New Escalation Policy'} width={1060} height="min(940px, 100%)">
      {body}
    </Modal>
  );
}

function Editor({ id, routeTeam, initial, usedBy, onClose }: { id?: string; routeTeam?: string; initial: OnCallPolicyInput; usedBy: string[]; onClose: () => void }) {
  const [p, setP] = useState(initial);
  const [tagText, setTagText] = useState((initial.tags ?? []).join(', '));
  const [resolveOn, setResolveOn] = useState(initial.resolveAfterMinutes != null);
  const [repeatOn, setRepeatOn] = useState((initial.repeat ?? 0) > 0);
  const save = useSaveOnCallPolicy(id);
  const route = useSetOnCallRouting();
  const remove = useDeleteOnCallPolicy();
  const users = useUsers({ offset: 0, limit: 200, statuses: ['active'] });
  const schedules = useOnCallSchedules();
  const teams = useOnCallTeams();
  const names: Record<OnCallTargetKind, { value: string; label: string }[]> = {
    user: (users.data?.users ?? []).map((u) => ({ value: u.id, label: u.name })),
    schedule: (schedules.data ?? []).map((s) => ({ value: s.id, label: s.name })),
    team: (teams.data ?? []).map((t) => ({ value: t.handle, label: t.name })),
  };
  const nameOf = (kind: OnCallTargetKind, value: string) => names[kind].find((o) => o.value === value)?.label ?? value;
  const setStep = (i: number, s: Partial<OnCallStep>) => setP({ ...p, steps: p.steps.map((x, j) => (j === i ? { ...x, ...s } : x)) });
  const insertStep = (at: number) => setP({ ...p, steps: [...p.steps.slice(0, at), { targets: [], escalateAfterMinutes: 5 }, ...p.steps.slice(at)] });
  const input: OnCallPolicyInput = {
    ...p,
    repeat: repeatOn ? (p.repeat ?? 0) : 0,
    resolveAfterMinutes: resolveOn ? (p.resolveAfterMinutes ?? 240) : null,
    tags: tagText
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean),
  };
  const problem = policyProblem(input);
  const busy = save.isPending || route.isPending || remove.isPending;
  const submit = () =>
    save.mutate(input, {
      onSuccess: (saved) => (routeTeam ? route.mutate({ handle: routeTeam, policyId: saved.id }, { onSuccess: onClose }) : onClose()),
    });
  const addStep = (at: number) => (
    <div className="onc-flow-link">
      <button type="button" className="btn btn-sm" disabled={p.steps.length >= MAX_STEPS} onClick={() => insertStep(at)}>
        <Plus size={13} /> Add Step
      </button>
    </div>
  );
  const error = save.error ?? route.error ?? remove.error;
  return (
    <form
      className="onc-policy-modal"
      onSubmit={(e) => {
        e.preventDefault();
        if (!problem) submit();
      }}
    >
      <div className="onc-policy-modal-head">
        <h3>{id ? initial.name || 'Escalation Policy' : 'New Escalation Policy'}</h3>
        <button type="button" className="icon-plain" onClick={onClose} title="Close">
          <X size={18} />
        </button>
      </div>
      <div className="onc-policy-modal-body">
        <div className="onc-policy-details">
          <label className="form-label onc-span-2">
            <span>
              Name <span className="onc-required">*</span>
            </span>
            <input autoFocus value={p.name} maxLength={200} onChange={(e) => setP({ ...p, name: e.target.value })} placeholder="e.g. Checkout – Primary" />
          </label>
          <div className="form-label">
            Teams
            <Select searchable value={p.team ?? ''} onChange={(t) => setP({ ...p, team: t || null })} options={[{ value: '', label: 'Select a team' }, ...names.team]} />
          </div>
          <label className="form-label">
            Tags
            <input value={tagText} onChange={(e) => setTagText(e.target.value)} placeholder="Add tags (e.g. env:prod), separated by commas" />
          </label>
        </div>
        <div className="onc-flow">
          <div className="onc-flow-card">
            <span className="onc-flow-icon onc-flow-trigger">
              <CircleAlert size={18} />
            </span>
            <span>Page is triggered</span>
          </div>
          {addStep(0)}
          {p.steps.map((s, i) => (
            <div key={i}>
              <div className="onc-flow-card onc-flow-step">
                <div className="onc-flow-step-head">
                  <span className="onc-step-num">{i + 1}</span>
                  <span className="onc-flow-mode">Notify all recipients</span>
                  {p.steps.length > 1 && (
                    <button type="button" className="icon-btn onc-step-remove" title="Remove step" onClick={() => setP({ ...p, steps: p.steps.filter((_, j) => j !== i) })}>
                      <Trash2 size={13} />
                    </button>
                  )}
                </div>
                <div className="onc-recipient-add">
                  <span className="onc-recipient-label">Add recipient</span>
                  <Select
                    searchable
                    value=""
                    onChange={(v) => {
                      const [kind, ...rest] = v.split(':');
                      const value = rest.join(':');
                      if (value && !s.targets.some((t) => t.kind === kind && t.id === value)) setStep(i, { targets: [...s.targets, { kind: kind as OnCallTargetKind, id: value }] });
                    }}
                    options={[
                      { value: '', label: 'Search users, teams, or schedules' },
                      ...(['user', 'team', 'schedule'] as const).flatMap((kind) =>
                        names[kind].map((o) => {
                          const Icon = KIND_ICON[kind];
                          return { value: `${kind}:${o.value}`, label: o.label, icon: <Icon size={13} />, hint: KIND_LABEL[kind] };
                        }),
                      ),
                    ]}
                  />
                </div>
                {s.targets.length ? (
                  <div className="onc-targets">
                    {s.targets.map((t, ti) => {
                      const Icon = KIND_ICON[t.kind];
                      return (
                        <span key={`${t.kind}-${t.id}`} className="onc-target">
                          <Icon size={13} />
                          {nameOf(t.kind, t.id)}
                          <button type="button" className="icon-plain" title="Remove" onClick={() => setStep(i, { targets: s.targets.filter((_, j) => j !== ti) })}>
                            <X size={12} />
                          </button>
                        </span>
                      );
                    })}
                  </div>
                ) : (
                  <div className="onc-flow-empty">
                    <b>No responders added</b>
                    <span className="muted">Search and add users, teams, or schedules to the step</span>
                  </div>
                )}
                <div className="onc-inline">
                  <Clock size={15} />
                  <span>Escalate after</span>
                  <Select value={String(s.escalateAfterMinutes)} onChange={(v) => setStep(i, { escalateAfterMinutes: Number(v) })} options={minuteChoices(ESCALATE_MINUTES, s.escalateAfterMinutes)} />
                  <span>if not acknowledged</span>
                </div>
              </div>
              {addStep(i + 1)}
            </div>
          ))}
          <div className="onc-flow-card">
            <span className="onc-flow-icon onc-flow-repeat">
              <Repeat size={16} />
            </span>
            <label className="onc-inline">
              <input type="checkbox" checked={repeatOn} onChange={(e) => (setRepeatOn(e.target.checked), e.target.checked && !p.repeat && setP({ ...p, repeat: 1 }))} />
              <span>Repeat all the steps above up to</span>
              <input type="number" className="onc-num" min={1} max={10} disabled={!repeatOn} value={repeatOn ? (p.repeat ?? 0) : 0} onChange={(e) => setP({ ...p, repeat: Number(e.target.value) })} />
              <span>times</span>
            </label>
          </div>
          <div className="onc-flow-link" />
          <div className="onc-flow-card">
            <span className="onc-flow-icon onc-flow-resolve">
              <CheckCircle2 size={16} />
            </span>
            <label className="onc-inline">
              <input type="checkbox" checked={resolveOn} onChange={(e) => setResolveOn(e.target.checked)} />
              <span>Automatically set the page to</span>
              <span className="onc-status onc-status-resolved">RESOLVED</span>
              <Select
                value={String(p.resolveAfterMinutes ?? 240)}
                onChange={(v) => (setResolveOn(true), setP({ ...p, resolveAfterMinutes: Number(v) }))}
                options={minuteChoices(RESOLVE_MINUTES, p.resolveAfterMinutes ?? 240)}
              />
              <span>after it is triggered</span>
            </label>
          </div>
          <div className="onc-flow-tail">
            <button type="button" className="btn" disabled={p.steps.length >= MAX_STEPS} onClick={() => insertStep(p.steps.length)}>
              <Plus size={14} /> Add Step
            </button>
            <span className="muted onc-hint">Responders are notified by email, Slack or webhook, following their On-Call profiles. There is no phone or SMS delivery.</span>
          </div>
        </div>
      </div>
      <div className="onc-policy-modal-foot">
        {id && (
          <button
            type="button"
            className="btn btn-danger"
            disabled={busy}
            onClick={() =>
              confirm(`Delete ${initial.name}?${usedBy.length ? ` ${usedBy.map((t) => `@${t}`).join(', ')} will page every member instead.` : ''}`) && remove.mutate(id, { onSuccess: onClose })
            }
          >
            <Trash2 size={14} /> Delete
          </button>
        )}
        {error ? <p className="form-error">{errorText(error)}</p> : problem && <span className="muted onc-hint">{problem}</span>}
        <button type="button" className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn btn-primary" disabled={!!problem || busy}>
          {id ? 'Save' : 'Create Escalation Policy'}
        </button>
      </div>
    </form>
  );
}
