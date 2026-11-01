import { useDeferredValue, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { PanelLeftClose, PanelLeftOpen, PhoneCall, Plus, Search, UserRound, X } from 'lucide-react';
import type { OnCallPageEvent } from '../../api/onCall/client';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { fmtDateTime } from '../incidents/queries';
import { EmptyState, PageStatusBadge, PagesTable, People, TeamFacet, errorText } from './common';
import { useCreateOnCallPage, useOnCallPage, useOnCallPages, useOnCallPolicies, useOnCallTeams, usePageAction } from './queries';

const STATUS_TABS = [
  ['active', 'Active'],
  ['triggered', 'Triggered'],
  ['acknowledged', 'Acknowledged'],
  ['resolved', 'Resolved'],
  ['all', 'All'],
] as const;
type StatusTab = (typeof STATUS_TABS)[number][0];

/** On-Call → Pages (Past 7 Days by default). */
export function PagesPage() {
  return (
    <TimeProvider initial="1w">
      <PagesView />
    </TimeProvider>
  );
}

function PagesView() {
  const navigate = useNavigate();
  const { id } = useParams();
  const [params, setParams] = useSearchParams();
  const set = (k: string, v: string | null) => {
    const next = new URLSearchParams(params);
    if (v) next.set(k, v);
    else next.delete(k);
    setParams(next, { replace: true });
  };
  const query = params.get('q') ?? '';
  const deferred = useDeferredValue(query);
  const status = (params.get('status') as StatusTab) || 'active';
  const mine = params.get('mine') === '1';
  const teams = params.getAll('team');
  const [controls, setControls] = useState(true);
  const [creating, setCreating] = useState(false);
  const { range } = useTime();
  const minute = 60_000;
  const q = useOnCallPages({ status, teams, mine, query: deferred, from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute });
  const allTeams = useOnCallTeams();
  const setTeams = (list: string[]) => {
    const next = new URLSearchParams(params);
    next.delete('team');
    for (const t of list) next.append('team', t);
    setParams(next, { replace: true });
  };
  const counts = q.data?.counts;
  return (
    <div className="onc-pages">
      <div className="onc-search-row">
        <div className="mon-filter onc-search">
          <Search size={16} />
          <input placeholder="Search pages by title" value={query} onChange={(e) => set('q', e.target.value || null)} />
        </div>
        <button className="btn btn-primary" onClick={() => setCreating(true)}>
          <Plus size={15} /> Page
        </button>
      </div>
      <div className={`onc-split${controls ? '' : ' no-controls'}`}>
        {controls && (
          <aside className="onc-side">
            <nav className="onc-side-nav">
              <button className={!mine ? 'active' : ''} onClick={() => set('mine', null)}>
                <PhoneCall size={15} /> All Pages
              </button>
              <button className={mine ? 'active' : ''} onClick={() => set('mine', '1')}>
                <UserRound size={15} /> My Pages
              </button>
            </nav>
            <TeamFacet teams={(allTeams.data ?? []).map((t) => ({ handle: t.handle, name: t.name }))} selected={teams} onChange={setTeams} defaultOpen={teams.length > 0} />
          </aside>
        )}
        <div className="onc-main">
          <div className="onc-main-bar">
            <button className="btn btn-sm" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={14} /> : <PanelLeftOpen size={14} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <div className="onc-status-tabs" role="tablist">
              {STATUS_TABS.map(([k, label]) => (
                <button key={k} role="tab" aria-selected={status === k} className={status === k ? 'active' : ''} onClick={() => set('status', k === 'active' ? null : k)}>
                  {label}
                  <span className="onc-count">{counts ? counts[k] : '–'}</span>
                </button>
              ))}
            </div>
            <div className="onc-main-time">
              <TimePicker compact />
            </div>
          </div>
          {q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="onc-table-wrap">
              <PagesTable
                pages={q.data?.pages ?? []}
                onOpen={(p) => navigate({ pathname: `/on-call/pages/${p.id}`, search: params.toString() })}
                empty={
                  <EmptyState
                    icon={PhoneCall}
                    title={q.isPending ? 'Loading pages…' : 'No Pages found'}
                    hint={q.isPending ? undefined : 'Pages created in the selected time and filters appear here.'}
                  />
                }
              />
            </div>
          )}
        </div>
      </div>
      {id && <PageDrawer id={Number(id)} onClose={() => navigate({ pathname: '/on-call/pages', search: params.toString() })} />}
      {creating && <NewPageModal onClose={() => setCreating(false)} onCreated={(pid) => navigate(`/on-call/pages/${pid}`)} />}
    </div>
  );
}

const EVENT_LABEL: Record<string, string> = {
  triggered: 'Triggered',
  notified: 'Notified',
  escalated: 'Escalated',
  acknowledged: 'Acknowledged',
  resolved: 'Resolved',
  reassigned: 'Reassigned',
  failed: 'Not delivered',
};

function TimelineEntry({ e }: { e: OnCallPageEvent }) {
  return (
    <li className={`onc-tl-item onc-tl-${e.kind}`}>
      <span className="onc-tl-dot" />
      <div>
        <div className="onc-tl-head">
          <b>{EVENT_LABEL[e.kind] ?? e.kind}</b>
          <span className="muted">
            {fmtDateTime(e.createdAt)} · {e.actor}
          </span>
        </div>
        {e.body && <div className="onc-tl-body">{e.body}</div>}
      </div>
    </li>
  );
}

function PageDrawer({ id, onClose }: { id: number; onClose: () => void }) {
  const q = useOnCallPage(id);
  const act = usePageAction();
  const teams = useOnCallTeams();
  const [reassign, setReassign] = useState<string | null>(null);
  const p = q.data?.page;
  return (
    <div className="onc-drawer-backdrop" onClick={onClose}>
      <aside className="onc-drawer" onClick={(e) => e.stopPropagation()} aria-label="Page">
        <div className="onc-drawer-head">
          <PhoneCall size={18} />
          <span className="muted">Page #{id}</span>
          <button className="icon-plain" title="Close" onClick={onClose}>
            <X size={18} />
          </button>
        </div>
        {q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : !p ? (
          <p className="muted onc-pad">Loading…</p>
        ) : (
          <>
            <h2 className="onc-drawer-title">{p.title}</h2>
            <div className="onc-drawer-meta">
              <PageStatusBadge status={p.status} />
              <span className={`onc-urgency onc-urgency-${p.urgency}`}>{p.urgency === 'high' ? 'High urgency' : 'Low urgency'}</span>
              {p.nextEscalationAt && <span className="muted">Next escalation {fmtDateTime(p.nextEscalationAt)}</span>}
            </div>
            <div className="onc-drawer-actions">
              <button className="btn btn-primary" disabled={!q.data?.canRespond || p.status !== 'triggered' || act.isPending} onClick={() => act.mutate({ id, action: 'acknowledge' })}>
                Acknowledge
              </button>
              <button className="btn" disabled={!q.data?.canRespond || p.status === 'resolved' || act.isPending} onClick={() => act.mutate({ id, action: 'resolve' })}>
                Resolve
              </button>
              <button className="btn" disabled={p.status === 'resolved'} onClick={() => setReassign(p.team ?? '')}>
                Reassign
              </button>
            </div>
            {reassign != null && (
              <div className="onc-reassign">
                <Select label="Team" value={reassign} onChange={setReassign} options={(teams.data ?? []).map((t) => ({ value: t.handle, label: t.name, hint: t.policy?.name ?? 'every member' }))} />
                <button
                  className="btn btn-primary btn-sm"
                  disabled={!reassign || act.isPending}
                  onClick={() => act.mutate({ id, action: 'reassign', reassign: { team: reassign } }, { onSuccess: () => setReassign(null) })}
                >
                  Page Team
                </button>
                <button className="btn btn-sm" onClick={() => setReassign(null)}>
                  Cancel
                </button>
              </div>
            )}
            {act.error && <p className="form-error">{errorText(act.error)}</p>}
            <dl className="onc-facts">
              <dt>Paged to</dt>
              <dd>{p.team ? `@${p.team}` : '—'}</dd>
              <dt>Escalation policy</dt>
              <dd>{p.policy ? <Link to={`/on-call/escalation-policies/${p.policy.id}`}>{p.policy.name}</Link> : p.team ? 'None: every member of the team' : '—'}</dd>
              <dt>Source</dt>
              <dd>
                {p.source === 'incident' && p.incidentId != null ? (
                  <Link to={`/incidents/${p.incidentId}`}>Incident IR-{p.incidentId}</Link>
                ) : p.source === 'monitor' && p.monitorId != null ? (
                  <Link to={`/monitors/${p.monitorId}`}>Monitor #{p.monitorId}</Link>
                ) : (
                  `Created by ${p.createdBy}`
                )}
              </dd>
              <dt>Responders</dt>
              <dd>
                <People users={p.responders} max={6} empty="Nobody notified yet" />
              </dd>
              <dt>Created</dt>
              <dd>{fmtDateTime(p.createdAt)}</dd>
              {p.acknowledgedAt && (
                <>
                  <dt>Acknowledged</dt>
                  <dd>
                    {fmtDateTime(p.acknowledgedAt)} by {p.acknowledgedBy}
                  </dd>
                </>
              )}
              {p.resolvedAt && (
                <>
                  <dt>Resolved</dt>
                  <dd>
                    {fmtDateTime(p.resolvedAt)} by {p.resolvedBy}
                  </dd>
                </>
              )}
            </dl>
            {p.description && <pre className="onc-description">{p.description}</pre>}
            <h3 className="onc-drawer-sub">Timeline</h3>
            <ol className="onc-timeline">
              {[...(q.data?.timeline ?? [])].reverse().map((e) => (
                <TimelineEntry key={e.id} e={e} />
              ))}
            </ol>
          </>
        )}
      </aside>
    </div>
  );
}

export function NewPageModal({ onClose, onCreated, team: initialTeam = '' }: { onClose: () => void; onCreated?: (id: number) => void; team?: string }) {
  const teams = useOnCallTeams();
  const policies = useOnCallPolicies();
  const create = useCreateOnCallPage();
  const [target, setTarget] = useState<'team' | 'policy'>('team');
  const [team, setTeam] = useState(initialTeam);
  const [policyId, setPolicyId] = useState('');
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [urgency, setUrgency] = useState<'high' | 'low'>('high');
  const chosen = (teams.data ?? []).find((t) => t.handle === team);
  const ok = title.trim() && (target === 'team' ? team : policyId);
  return (
    <Modal onClose={onClose} label="Page" width={600} height="auto">
      <form
        className="small-modal onc-modal"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate(
            { title, description, urgency, team: target === 'team' ? team : null, policyId: target === 'policy' ? policyId : null },
            {
              onSuccess: (p) => {
                onClose();
                onCreated?.(p.id);
              },
            },
          );
        }}
      >
        <div className="modal-title-row">
          <h3>Page</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <div className="onc-form-row">
          <span className="form-label">Page a</span>
          <div className="btn-group">
            <button type="button" className={`btn${target === 'team' ? ' btn-selected' : ''}`} onClick={() => setTarget('team')}>
              Team
            </button>
            <button type="button" className={`btn${target === 'policy' ? ' btn-selected' : ''}`} onClick={() => setTarget('policy')}>
              Escalation policy
            </button>
          </div>
        </div>
        {target === 'team' ? (
          <>
            <Select label="Team" value={team} onChange={setTeam} options={[{ value: '', label: 'Select a team…' }, ...(teams.data ?? []).map((t) => ({ value: t.handle, label: t.name }))]} />
            {chosen && (
              <p className="muted onc-hint">{chosen.policy ? `Follows ${chosen.policy.name}.` : `@${chosen.handle} has no escalation policy: all ${chosen.memberCount} members are notified.`}</p>
            )}
          </>
        ) : (
          <Select
            label="Escalation policy"
            value={policyId}
            onChange={setPolicyId}
            options={[{ value: '', label: 'Select a policy…' }, ...(policies.data ?? []).map((p) => ({ value: p.id, label: p.name }))]}
          />
        )}
        <label className="form-label">
          Title
          <input value={title} onChange={(e) => setTitle(e.target.value)} required maxLength={300} placeholder="Checkout error rate above 5%" />
        </label>
        <label className="form-label">
          Description
          <textarea rows={4} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What's happening, and where to look" />
        </label>
        <div className="onc-form-row">
          <span className="form-label">Urgency</span>
          <div className="btn-group">
            <button type="button" className={`btn${urgency === 'high' ? ' btn-selected' : ''}`} onClick={() => setUrgency('high')}>
              High
            </button>
            <button type="button" className={`btn${urgency === 'low' ? ' btn-selected' : ''}`} onClick={() => setUrgency('low')}>
              Low
            </button>
          </div>
        </div>
        <p className="muted onc-hint">Responders are notified by email, Slack or webhook, as set in their On-Call profiles. There is no phone or SMS delivery.</p>
        {create.error && <p className="form-error">{errorText(create.error)}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!ok || create.isPending}>
            Page
          </button>
        </div>
      </form>
    </Modal>
  );
}
