import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Inbox, ListChecks, PanelLeft, PanelLeftClose, Search } from 'lucide-react';
import { workItems, type WorkItem } from '../../api/events/client';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { SourceIcon } from './eventVisuals';

type View = 'all' | 'open' | 'closed';

/**
 * Event Management → Triage Inbox: the range's warning and error events correlated into work items,
 * one per affected object (Kubernetes object, host…). Open while its events keep coming.
 */
export function TriageInbox() {
  const { range } = useTime();
  const [showProjects, setShowProjects] = useState(true);
  const [query, setQuery] = useState('');
  const [view, setView] = useState<View>('open');
  const [expanded, setExpanded] = useState<string | null>(null);
  const q = useQuery({ queryKey: ['events', 'work-items', range, query], queryFn: () => workItems({ range, query }) });
  const page = q.data;
  const shown = (page?.items ?? []).filter((i) => view === 'all' || i.state === view);
  const views: { key: View; label: string; count: number | undefined }[] = [
    { key: 'all', label: 'All Work Items', count: page?.items.length },
    { key: 'open', label: 'Open', count: page?.open },
    { key: 'closed', label: 'Closed', count: page?.closed },
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
            <p>Warning and error events in the time range become work items, one per affected object.</p>
          </div>
        ) : (
          <div className="ev-wi-list">
            {shown.map((i) => (
              <WorkItemRow key={i.id} item={i} open={expanded === i.id} onToggle={() => setExpanded((e) => (e === i.id ? null : i.id))} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function WorkItemRow({ item, open, onToggle }: { item: WorkItem; open: boolean; onToggle: () => void }) {
  return (
    <div className={`ev-wi ev-wi-${item.status}${open ? ' open' : ''}`}>
      <button className="ev-wi-head" onClick={onToggle}>
        <SourceIcon source={item.source} size={18} />
        <span className="ev-wi-main">
          <span className="ev-wi-title">
            {item.reason && <span className={`ev-wi-reason ev-wi-${item.status}`}>{item.reason}</span>} {item.title}
          </span>
          <span className="ev-wi-tags">
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
        </div>
      )}
    </div>
  );
}
