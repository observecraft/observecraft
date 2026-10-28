import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Inbox, ListChecks, PanelLeft, PanelLeftClose, Search, Wrench } from 'lucide-react';
import { listWorkItemAttributes, setWorkItemAttribute, workItems, type WorkItem, type WorkItemAttribute } from '../../api/events/client';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { SourceIcon } from './eventVisuals';
import { useAccess } from '../settings/queries';
import { ATTRIBUTES_KEY } from './settings/WorkItemAttributesPage';

import { inView, type InboxView as View } from '../../lib/eventCorrelation';

/**
 * Event Management → Triage Inbox: the range's warning and error events correlated into work items, by
 * the correlators of Event Management settings, else one per affected object (Kubernetes object,
 * host…). Open while its events keep coming; suppressed when a maintenance window covers them.
 */
export function TriageInbox() {
  const { range } = useTime();
  const [showProjects, setShowProjects] = useState(true);
  const [query, setQuery] = useState('');
  const [view, setView] = useState<View>('open');
  const [expanded, setExpanded] = useState<string | null>(null);
  const q = useQuery({ queryKey: ['events', 'work-items', range, query], queryFn: () => workItems({ range, query }) });
  const attributes = useQuery({ queryKey: ATTRIBUTES_KEY, queryFn: listWorkItemAttributes });
  const page = q.data;
  const shown = (page?.items ?? []).filter((i) => inView(i, view));
  const views: { key: View; label: string; count: number | undefined }[] = [
    { key: 'all', label: 'All Work Items', count: page?.items.length },
    { key: 'open', label: 'Open', count: page?.open },
    { key: 'closed', label: 'Closed', count: page?.closed },
    { key: 'suppressed', label: 'Suppressed', count: page?.suppressed },
  ];

  return (
    <div className="ev-inbox">
      {showProjects && (
        <aside className="ev-projects">
          <button className="ev-projects-toggle" onClick={() => setShowProjects(false)}>
            <PanelLeftClose size={16} /> Hide Projects
          </button>
          <div className="ev-proj-title">Correlation</div>
          {views.map((v) => (
            <button key={v.key} className={`ev-proj-row ${v.key === 'all' ? 'ev-proj-sub' : 'ev-proj-sub2'}${view === v.key ? ' active' : ''}`} onClick={() => setView(v.key)}>
              {v.key === 'all' && <ListChecks size={16} />} {v.label} <span className="ev-proj-end">{v.count ?? '–'}</span>
            </button>
          ))}
        </aside>
      )}
      <div className="ev-inbox-main">
        <div className="ev-inbox-bar">
          {!showProjects && (
            <button className="btn" onClick={() => setShowProjects(true)} title="Show projects">
              <PanelLeft size={15} />
            </button>
          )}
          <label className="ev-inbox-search">
            <Search size={15} className="muted" />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Filter the correlated events, e.g. source:kubernetes kube_namespace:shop" />
          </label>
          <TimePicker compact />
        </div>
        {q.isPending ? (
          <Loading label="Correlating events…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : shown.length === 0 ? (
          <div className="ev-inbox-empty">
            <Inbox size={96} strokeWidth={1.5} className="ev-inbox-empty-icon" />
            <h3>No {view === 'all' ? '' : view} work items</h3>
            <p>
              Warning and error events in the time range become work items: grouped by your <Link to="/event/settings/correlators">correlators</Link>, else one per affected object.
            </p>
          </div>
        ) : (
          <div className="ev-wi-list">
            {shown.map((i) => (
              <WorkItemRow key={i.id} item={i} attributes={attributes.data ?? []} open={expanded === i.id} onToggle={() => setExpanded((e) => (e === i.id ? null : i.id))} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function WorkItemRow({ item, attributes, open, onToggle }: { item: WorkItem; attributes: WorkItemAttribute[]; open: boolean; onToggle: () => void }) {
  return (
    <div className={`ev-wi ev-wi-${item.status}${open ? ' open' : ''}`}>
      <button className="ev-wi-head" onClick={onToggle}>
        <SourceIcon source={item.source} size={18} />
        <span className="ev-wi-main">
          <span className="ev-wi-title">
            {item.reason && <span className={`ev-wi-reason ev-wi-${item.status}`}>{item.reason}</span>} {item.title}
          </span>
          <span className="ev-wi-tags">
            {item.correlator && <span className="ev-wi-correlator">Correlated by {item.correlator}</span>}
            {item.maintenance && (
              <span className="ev-wi-maintenance" title="Every event of this work item falls in a maintenance window">
                <Wrench size={12} /> {item.maintenance}
              </span>
            )}
            {item.tags.map((t) => (
              <span key={t} className="tag-chip">
                {t}
              </span>
            ))}
          </span>
        </span>
        <span className={`ev-wi-state ev-wi-state-${item.state}`}>{item.state === 'open' ? 'Open' : 'Closed'}</span>
        <span className="ev-wi-count">{item.count.toLocaleString()} events</span>
        <span className="muted ev-wi-time">{timeAgo(item.lastSeen)}</span>
      </button>
      {open && (
        <div className="ev-wi-body">
          <p className="mono">{item.summary}</p>
          <p className="muted">
            First seen {new Date(item.firstSeen).toLocaleString()} · last seen {new Date(item.lastSeen).toLocaleString()}
          </p>
          {attributes.length > 0 && <AttributeFields item={item} attributes={attributes} />}
        </div>
      )}
    </div>
  );
}

/** The work item's attribute values, editable by Standard users and admins. */
function AttributeFields({ item, attributes }: { item: WorkItem; attributes: WorkItemAttribute[] }) {
  const qc = useQueryClient();
  const { canWrite } = useAccess();
  const save = useMutation({
    mutationFn: ({ attributeId, value }: { attributeId: string; value: string }) => setWorkItemAttribute(item.id, attributeId, value),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['events', 'work-items'] }),
  });
  return (
    <div className="ev-wi-attrs">
      {attributes.map((a) => {
        const value = item.attributes[a.id] ?? '';
        return (
          <label key={a.id} className="ev-wi-attr">
            <span className="muted">{a.name}</span>
            {a.kind === 'select' ? (
              <select value={value} disabled={!canWrite || save.isPending} onChange={(e) => save.mutate({ attributeId: a.id, value: e.target.value })}>
                <option value="">—</option>
                {a.options.map((o) => (
                  <option key={o} value={o}>
                    {o}
                  </option>
                ))}
              </select>
            ) : (
              <input
                defaultValue={value}
                disabled={!canWrite || save.isPending}
                placeholder="—"
                onBlur={(e) => e.target.value.trim() !== value && save.mutate({ attributeId: a.id, value: e.target.value })}
                onKeyDown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
              />
            )}
          </label>
        );
      })}
      {save.error && <p className="form-error">{(save.error as Error).message}</p>}
    </div>
  );
}
