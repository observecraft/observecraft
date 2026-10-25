import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowUpToLine, ChevronDown, ExternalLink, ThumbsDown, ThumbsUp, X } from 'lucide-react';
import type { IssueDetail } from '../../../api/errorTracking/types';
import { useTime, TimeProvider } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Menu } from '../../../components/ui/Dropdown';
import { formatLongDate } from '../../../lib/format';
import { AppChip, CrashBadge, RegressionBadge, STATUS_LABEL } from '../issueVisuals';
import { StatusSelect } from '../issueControls';
import { snapRange, useIssue, useUpdateIssue } from '../queries';
import { ActivityTab } from './ActivityTab';
import { ResolveTab } from './ResolveTab';
import { ReviewTab } from './ReviewTab';

const TABS = ['Review', 'Resolve', 'Activity'] as const;
type Tab = (typeof TABS)[number];

function issueUrl(id: string): string {
  return `${window.location.origin}/errors?issue=${encodeURIComponent(id)}`;
}

function toMarkdown(i: IssueDetail): string {
  return [
    `## ${i.errorType}`,
    '',
    i.message,
    '',
    `- **Service:** ${i.service}${i.application ? ` (${i.application.packageId ?? i.application.name})` : ''}`,
    `- **Status:** ${STATUS_LABEL[i.status]}`,
    `- **Occurrences (range):** ${i.count}`,
    `- **First seen:** ${formatLongDate(i.firstSeen)}${i.firstSeenVersion ? ` (${i.firstSeenVersion})` : ''}`,
    `- **Last seen:** ${formatLongDate(i.lastSeen)}${i.lastSeenVersion ? ` (${i.lastSeenVersion})` : ''}`,
    i.location ? `- **Location:** \`${i.location.function}\`${i.location.file ? ` in ${i.location.file}` : ''}` : '',
    '',
    `[Open in ObserveCraft](${issueUrl(i.id)})`,
  ]
    .filter((l) => l !== null)
    .join('\n');
}

/** Right-hand issue panel. Has its own time range (default: past day), independent of the list. */
export function IssuePanel({ id, onClose }: { id: string; onClose: () => void }) {
  return (
    <aside className="issue-panel" role="dialog" aria-label="Issue details">
      <TimeProvider initial="1d">
        <PanelBody id={id} onClose={onClose} />
      </TimeProvider>
    </aside>
  );
}

function PanelBody({ id, onClose }: { id: string; onClose: () => void }) {
  const { range: live } = useTime();
  const { from, to } = snapRange(live);
  const range = useMemo(() => ({ from, to }), [from, to]);
  const issueQ = useIssue(id, range);
  const update = useUpdateIssue();
  const [tab, setTab] = useState<Tab>('Review');
  const [copied, setCopied] = useState(false);
  const [showTop, setShowTop] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !(e.target as HTMLElement).closest('input, textarea')) onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  useEffect(() => {
    setTab('Review');
    scroller.current?.scrollTo({ top: 0 });
  }, [id]);

  if (issueQ.isPending) return <Loading label="Loading issue…" />;
  if (issueQ.isError) return <ErrorState error={issueQ.error} onRetry={() => issueQ.refetch()} />;
  const issue = issueQ.data;

  const copy = (text: string) => {
    navigator.clipboard?.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <div className={`issue-panel-scroll status-edge-${issue.status}`} ref={scroller} onScroll={(e) => setShowTop(e.currentTarget.scrollTop > 600)}>
      <header className="ip-header">
        <StatusSelect issueId={issue.id} status={issue.status} />
        <div className="ip-actions">
          <span className="muted-text">Give feedback</span>
          <button className="icon-plain" title="Helpful">
            <ThumbsUp size={16} />
          </button>
          <button className="icon-plain" title="Not helpful">
            <ThumbsDown size={16} />
          </button>
          <Menu
            trigger={
              <>
                Actions <ChevronDown size={14} />
              </>
            }
            items={[
              { label: 'Mark as reviewed', onClick: () => update.mutate({ id: issue.id, patch: { status: 'reviewed' } }) },
              { label: 'Mark as resolved', onClick: () => update.mutate({ id: issue.id, patch: { status: 'resolved' } }) },
              { label: 'Ignore', onClick: () => update.mutate({ id: issue.id, patch: { status: 'ignored' } }) },
              { label: 'Copy link', onClick: () => copy(issueUrl(issue.id)) },
            ]}
          />
          <div className="btn-group">
            <button className="btn" onClick={() => copy(toMarkdown(issue))}>
              {copied ? 'Copied!' : 'Copy as Markdown'}
            </button>
            <Menu
              trigger={<ChevronDown size={14} />}
              items={[
                { label: 'Copy as Markdown', onClick: () => copy(toMarkdown(issue)) },
                { label: 'Copy as JSON', onClick: () => copy(JSON.stringify(issue, null, 2)) },
              ]}
            />
          </div>
          <button className="icon-plain" title="Open in new tab" onClick={() => window.open(issueUrl(issue.id), '_blank')}>
            <ExternalLink size={16} />
          </button>
          <button className="icon-plain" title="Close (Esc)" onClick={onClose}>
            <X size={20} />
          </button>
        </div>
      </header>

      <div className="ip-title">
        <h2>{issue.errorType}</h2>
        <p>{issue.message}</p>
        <div className="issue-chips">
          <AppChip issue={issue} withName />
          {issue.isCrash && <CrashBadge />}
          {issue.isRegression && <RegressionBadge />}
        </div>
      </div>

      <nav className="ip-tabs">
        {TABS.map((t) => (
          <button key={t} className={t === tab ? 'active' : ''} onClick={() => setTab(t)}>
            {t}
            {t === 'Resolve' && <span className="badge-new">PREVIEW</span>}
          </button>
        ))}
      </nav>

      {tab === 'Review' && <ReviewTab issue={issue} range={range} isRefreshing={issueQ.isPlaceholderData} />}
      {tab === 'Resolve' && <ResolveTab issue={issue} />}
      {tab === 'Activity' && <ActivityTab issue={issue} />}

      {showTop && (
        <button className="back-to-top" onClick={() => scroller.current?.scrollTo({ top: 0, behavior: 'smooth' })}>
          <ArrowUpToLine size={14} /> Back to Top
        </button>
      )}
    </div>
  );
}
