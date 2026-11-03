import { useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { CalendarCheck, ChevronDown, ChevronRight, Megaphone, MessageSquare, PhoneCall, Plus, Video } from 'lucide-react';
import { SEVERITIES, SEVERITY_LABEL, type IncidentDetail, type IncidentStatus } from '../../api/incidents/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select } from '../../components/ui/Dropdown';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { timeAgo } from '../../lib/format';
import { useUsers } from '../settings/queries';
import { IncidentInvestigation } from '../assist/IssueAssist';
import { AutomationsTab, NotificationsTab, PagesTab, PostIncidentTab, ResponseTeamTab, TimelineTab } from './IncidentTabs';
import { durationLong, durationShort, fmtDateTime, useAddFollowup, useAddImpact, useIncident, usePages, useToggleFollowup, useUpdateIncident } from './queries';
import '../settings/settings.css';
import { NOTEBOOK_TEMPLATES, fromTemplate } from '../../api/notebooks/templates';
import { useCreateNotebook } from '../notebooks/queries';
import './incidents.css';

type Tab = 'overview' | 'timeline' | 'post' | 'team' | 'pages' | 'notifications' | 'automations';
const TABS: [Tab, string][] = [
  ['overview', 'Overview'],
  ['timeline', 'Timeline'],
  ['post', 'Post-Incident'],
  ['team', 'Response Team'],
  ['pages', 'Pages'],
  ['notifications', 'Notifications'],
  ['automations', 'Automations'],
];

const DETECTION_LABEL: Record<string, string> = { monitor: 'Monitor', customer: 'Customer', employee: 'Employee', automation: 'Automation', unknown: 'Unknown' };
const SEV_COLOR: Record<string, string> = { 'SEV-1': '#d6204e', 'SEV-2': '#e5532d', 'SEV-3': '#e8336a', 'SEV-4': '#f2a93b', 'SEV-5': '#8a94a6', UNKNOWN: '#8a94a6' };

export function IncidentPage() {
  const id = Number(useParams().id);
  const q = useIncident(id);
  if (q.isPending) return <Loading label="Loading incident…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <Body inc={q.data} />;
}

function Body({ inc }: { inc: IncidentDetail }) {
  const [params] = useSearchParams();
  const [tab, setTab] = useState<Tab>(() => (TABS.some(([t]) => t === params.get('tab')) ? (params.get('tab') as Tab) : 'overview'));
  const update = useUpdateIncident(inc.id);
  const users = useUsers({ query: '', statuses: ['active'], offset: 0, limit: 200 });
  const end = inc.resolvedAt ?? Date.now();
  const navigate = useNavigate();
  const createNotebook = useCreateNotebook();
  // A postmortem notebook from the template, titled and tagged after the incident.
  const postmortemNotebook = () => {
    const postmortem = NOTEBOOK_TEMPLATES.find((t) => t.id === 'postmortem')!;
    const draft = fromTemplate(postmortem, `IR-${inc.id} ${inc.title}`);
    createNotebook.mutate({ ...draft, tags: [...draft.tags, `incident:IR-${inc.id}`] }, { onSuccess: (nb) => navigate(`/notebooks/${nb.id}`) });
  };
  return (
    <div className="inc-detail">
      <div className="inc-crumbs">
        <Link to="/incidents">Incidents</Link>
        <ChevronRight size={14} />
        <span>{inc.incidentType}</span>
        <ChevronRight size={14} />
        <span>IR-{inc.id}</span>
      </div>
      <h1 className="inc-title">
        {inc.test && <span className="test-chip">TEST</span>}
        {inc.title}
      </h1>
      <div className="inc-bar">
        <Select<IncidentStatus>
          value={inc.status}
          onChange={(status) => update.mutate({ status })}
          options={[
            { value: 'active', label: 'ACTIVE' },
            { value: 'stable', label: 'STABLE' },
            { value: 'resolved', label: 'RESOLVED' },
          ]}
          triggerClassName={`inc-status-btn inc-bg-${inc.status}`}
        />
        <Select
          value={inc.severity}
          onChange={(severity) => update.mutate({ severity })}
          options={SEVERITIES.map((s) => ({ value: s, label: `${s} (${SEVERITY_LABEL[s]})` }))}
          trigger={() => (
            <span className="inc-sev-btn">
              <i style={{ background: SEV_COLOR[inc.severity] }} />
              <b>{inc.severity}</b> ({SEVERITY_LABEL[inc.severity]}) <ChevronDown size={14} />
            </span>
          )}
          triggerClassName="btn"
        />
        <span className="inc-bar-label">Commander</span>
        <Select
          value={inc.commander?.id ?? ''}
          onChange={(commanderId) => update.mutate({ commanderId })}
          options={(users.data?.users ?? []).map((u) => ({ value: u.id, label: u.name }))}
          trigger={() =>
            inc.commander ? (
              <span className="person-chip big">
                <UserAvatar id={inc.commander.id} name={inc.commander.name} size={26} /> {inc.commander.name}
              </span>
            ) : (
              <span className="muted">Unassigned</span>
            )
          }
          triggerClassName="icon-plain"
        />
        <span className="inc-bar-label">Responders</span>
        {inc.responders.length ? (
          <span className="inc-avatars" title={inc.responders.map((r) => r.name).join(', ')}>
            {inc.responders.map((r) => (
              <UserAvatar key={r.id} id={r.id} name={r.name} size={30} />
            ))}
          </span>
        ) : (
          <span className="muted">No one</span>
        )}
        <span className="inc-bar-label">Duration</span>
        <span className="age-chip" title={durationLong(end - inc.declaredAt)}>
          {durationShort(end - inc.declaredAt)}
        </span>
      </div>
      <div className="inc-actions-row">
        <button className="btn" disabled title="Connect a chat integration (Slack, Microsoft Teams) first">
          <MessageSquare size={14} /> Add Chat
        </button>
        <button className="btn" disabled title="Connect a video integration (Zoom, Google Meet) first">
          <Video size={14} /> Add Video Conference
        </button>
        <span className="inc-actions-sep" />
        <Menu
          align="left"
          trigger={
            <>
              Actions <ChevronDown size={14} />
            </>
          }
          items={[
            { label: 'Page Team', onClick: () => setTab('pages') },
            { label: 'Send Notification', onClick: () => setTab('notifications') },
            { label: 'Trigger Automation', onClick: () => setTab('automations') },
            { label: 'Open Postmortem', onClick: () => setTab('post') },
            { label: inc.customerImpact ? 'Clear customer impact' : 'Mark customer impact', onClick: () => update.mutate({ customerImpact: !inc.customerImpact }) },
            { label: 'Create postmortem notebook', onClick: postmortemNotebook, disabled: createNotebook.isPending },
            { label: 'Copy link', onClick: () => navigator.clipboard?.writeText(window.location.href) },
          ]}
        />
      </div>
      {update.isError && <p className="form-error inc-pad">{(update.error as Error).message}</p>}
      {createNotebook.isError && <p className="form-error inc-pad">{createNotebook.error.message}</p>}
      <nav className="settings-tabs inc-tabs">
        {TABS.map(([t, label]) => (
          <button key={t} className={tab === t ? 'active' : ''} onClick={() => setTab(t)}>
            {label}
          </button>
        ))}
      </nav>
      <div className="inc-tab-body">
        {tab === 'overview' ? (
          <Overview inc={inc} onTab={setTab} />
        ) : tab === 'timeline' ? (
          <TimelineTab inc={inc} />
        ) : tab === 'team' ? (
          <ResponseTeamTab inc={inc} />
        ) : tab === 'notifications' ? (
          <NotificationsTab inc={inc} />
        ) : tab === 'post' ? (
          <PostIncidentTab inc={inc} />
        ) : tab === 'pages' ? (
          <PagesTab inc={inc} />
        ) : (
          <AutomationsTab inc={inc} />
        )}
      </div>
    </div>
  );
}

/** Pages sent for the incident, or a button to page a team. */
function PagesCard({ inc, onTab }: { inc: IncidentDetail; onTab: (t: Tab) => void }) {
  const pages = usePages(inc.id);
  const n = pages.data?.length ?? 0;
  return (
    <section className="settings-card">
      <div className="card-head">
        <PhoneCall size={18} /> <b>{n ? `${n} page${n === 1 ? '' : 's'} sent` : 'No pages sent'}</b>
        <button className="btn btn-sm inc-card-btn" onClick={() => onTab('pages')}>
          <PhoneCall size={14} /> {n ? 'View' : 'Page'}
        </button>
      </div>
    </section>
  );
}

function Notification({ body, at }: { body: string; at: number }) {
  return (
    <div className="inc-notif">
      <div>{body}</div>
      <div className="muted">{timeAgo(at)}</div>
    </div>
  );
}

function Editable({ value, placeholder, onSave }: { value: string; placeholder: string; onSave: (v: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  if (!editing)
    return (
      <button className={`inc-editable${value ? '' : ' placeholder'}`} onClick={() => (setDraft(value), setEditing(true))}>
        {value || placeholder}
      </button>
    );
  return (
    <div className="inc-edit">
      <textarea rows={4} value={draft} onChange={(e) => setDraft(e.target.value)} autoFocus />
      <div className="form-actions">
        <button className="btn" onClick={() => setEditing(false)}>
          Cancel
        </button>
        <button
          className="btn btn-primary"
          onClick={() => {
            onSave(draft);
            setEditing(false);
          }}
        >
          Save
        </button>
      </div>
    </div>
  );
}

function ListEdit({ values, onSave }: { values: string[]; onSave: (v: string[]) => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  if (!editing)
    return (
      <button className={`inc-editable${values.length ? '' : ' placeholder'}`} onClick={() => (setDraft(values.join(', ')), setEditing(true))}>
        {values.length
          ? values.map((v) => (
              <span key={v} className="prop-chip">
                {v}
              </span>
            ))
          : 'Select values'}
      </button>
    );
  return (
    <form
      className="inc-edit"
      onSubmit={(e) => {
        e.preventDefault();
        onSave(
          draft
            .split(',')
            .map((s) => s.trim())
            .filter(Boolean),
        );
        setEditing(false);
      }}
    >
      <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="comma-separated" autoFocus />
    </form>
  );
}

function Overview({ inc, onTab }: { inc: IncidentDetail; onTab: (t: Tab) => void }) {
  const update = useUpdateIncident(inc.id);
  const addImpact = useAddImpact(inc.id);
  const addFollowup = useAddFollowup(inc.id);
  const toggleFollowup = useToggleFollowup(inc.id);
  const [impact, setImpact] = useState<string | null>(null);
  const [followup, setFollowup] = useState<string | null>(null);
  const timestamps: [string, string][] = [
    ['Detected', fmtDateTime(inc.detectedAt)],
    ['Declared', fmtDateTime(inc.declaredAt)],
    ...(inc.resolvedAt ? ([['Resolved', fmtDateTime(inc.resolvedAt)]] as [string, string][]) : []),
    ...(inc.acknowledgedAt ? ([['Time to Internal Response', durationLong(inc.acknowledgedAt - inc.declaredAt)]] as [string, string][]) : []),
    ...(inc.resolvedAt
      ? ([
          ['Time to Resolve', durationLong(inc.resolvedAt - inc.declaredAt)],
          ['Time from Detect to Resolve', durationLong(inc.resolvedAt - inc.detectedAt)],
        ] as [string, string][])
      : []),
  ];
  return (
    <div className="inc-overview">
      <div className="inc-left">
        <section className="settings-card inc-life">
          <div className="life-step">
            <i className="life-dot gray" />
            <b>DETECTED</b> <span className="muted">{fmtDateTime(inc.detectedAt, false)}</span>
          </div>
          <div className="life-step">
            <i className="life-dot red" />
            <b className="tone-red">DECLARED</b> <span className="muted">{fmtDateTime(inc.declaredAt, false)}</span>
            <div className="muted life-sub">by {inc.declaredBy}</div>
          </div>
          {inc.stableAt && inc.status !== 'active' && (
            <div className="life-step">
              <i className="life-dot orange" />
              <b className="tone-orange">STABLE</b> <span className="muted">{fmtDateTime(inc.stableAt, false)}</span>
            </div>
          )}
          {inc.resolvedAt && (
            <div className="life-step">
              <i className="life-dot green" />
              <b className="tone-green">RESOLVED</b> <span className="muted">{fmtDateTime(inc.resolvedAt, false)}</span>
              <div className="muted life-sub">
                Time to Resolved:{' '}
                {durationLong(inc.resolvedAt - inc.declaredAt)
                  .split(' ')
                  .slice(0, 4)
                  .join(' ')}
              </div>
            </div>
          )}
        </section>
        <PagesCard inc={inc} onTab={onTab} />
        <section className="settings-card">
          <div className="card-head">
            <Megaphone size={18} /> <b>{inc.notifications.length ? 'Latest Notifications' : 'No notifications sent'}</b>
            {inc.notifications.length > 0 ? (
              <button className="link-btn" onClick={() => onTab('notifications')}>
                View all ({inc.notifications.length})
              </button>
            ) : (
              <button className="link-btn" onClick={() => onTab('notifications')}>
                <Plus size={14} /> Add
              </button>
            )}
          </div>
          {inc.notifications.length > 0 && <Notification body={inc.notifications[0].body} at={inc.notifications[0].createdAt} />}
        </section>
        <section className="settings-card">
          <div className="card-head">
            <CalendarCheck size={18} /> <b>{inc.followups.length ? 'Follow-ups' : 'No follow-ups'}</b>
            <span className="inc-card-links">
              {inc.followups.length > 0 && (
                <button className="link-btn" onClick={() => onTab('post')}>
                  View all ({inc.followups.length})
                </button>
              )}
              <button className="link-btn" onClick={() => setFollowup('')}>
                <Plus size={14} /> Add
              </button>
            </span>
          </div>
          {inc.followups.map((f) => (
            <label key={f.id} className="inc-check">
              <input type="checkbox" className="checkbox" checked={f.done} onChange={() => toggleFollowup.mutate({ followupId: f.id, done: !f.done })} />
              <span className={f.done ? 'done' : ''}>{f.title}</span>
            </label>
          ))}
          {followup !== null && (
            <form
              className="inc-edit"
              onSubmit={(e) => {
                e.preventDefault();
                if (followup.trim()) addFollowup.mutate(followup, { onSuccess: () => setFollowup(null) });
              }}
            >
              <input value={followup} onChange={(e) => setFollowup(e.target.value)} placeholder="What needs to happen?" autoFocus />
            </form>
          )}
        </section>
      </div>
      <section className="settings-card inc-center">
        <div className="inc-center-main">
          <h2>What happened</h2>
          <dl className="inc-props">
            <dt>Detection Method</dt>
            <dd>{DETECTION_LABEL[inc.detectionMethod] ?? inc.detectionMethod}</dd>
            <dt>Summary</dt>
            <dd>
              <Editable value={inc.summary} placeholder="Describe this incident in more detail..." onSave={(summary) => update.mutate({ summary })} />
            </dd>
          </dl>
          <h3 className="inc-h3">
            Impacts
            <button className="link-btn" onClick={() => setImpact('')}>
              <Plus size={14} /> Add
            </button>
          </h3>
          {inc.impacts.length === 0 && impact === null && <p className="muted">No impacts defined</p>}
          {inc.impacts.map((im) => (
            <div key={im.id} className="inc-impact">
              <b>{im.description}</b>
              <span className="muted">
                {fmtDateTime(im.startAt)} – {im.endAt ? fmtDateTime(im.endAt) : 'ongoing'}
              </span>
            </div>
          ))}
          {impact !== null && (
            <form
              className="inc-edit"
              onSubmit={(e) => {
                e.preventDefault();
                if (impact.trim()) addImpact.mutate({ description: impact, startAt: inc.detectedAt }, { onSuccess: () => setImpact(null) });
              }}
            >
              <input value={impact} onChange={(e) => setImpact(e.target.value)} placeholder="Who or what was affected?" autoFocus />
            </form>
          )}
          <h2>Investigation</h2>
          <IncidentInvestigation incidentId={inc.id} />
          <h2>Why it happened</h2>
          <dl className="inc-props">
            <dt>Root Cause</dt>
            <dd>
              <Editable value={inc.rootCause} placeholder="Describe the root cause and contributing factors..." onSave={(rootCause) => update.mutate({ rootCause })} />
            </dd>
          </dl>
        </div>
        <aside className="inc-center-side">
          <h4>Connections</h4>
          <p className="muted">No connections</p>
          <h4>Attributes</h4>
          <dl className="inc-attrs">
            <dt>Services</dt>
            <dd>
              <ListEdit values={inc.services} onSave={(services) => update.mutate({ services })} />
            </dd>
            <dt>Teams</dt>
            <dd>
              <ListEdit values={inc.teams} onSave={(teams) => update.mutate({ teams })} />
            </dd>
            <dt>Customer Impact</dt>
            <dd>{inc.customerImpact ? 'Yes' : 'No'}</dd>
          </dl>
          <h4>Timestamps</h4>
          <dl className="inc-attrs">
            {timestamps.map(([k, v]) => (
              <div key={k} className="inc-attr-row">
                <dt>{k}</dt>
                <dd>{v}</dd>
              </div>
            ))}
          </dl>
        </aside>
      </section>
    </div>
  );
}
