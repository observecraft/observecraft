import { memo } from 'react';
import type { Issue } from '../../api/errorTracking/types';
import { timeAgo } from '../../lib/format';
import { AssigneeSelect, CauseSelect, StatusSelect } from './issueControls';
import { AppChip, CrashBadge, PlatformIcon, RegressionBadge, Sparkline } from './issueVisuals';

/** "27 days old", "about 1 year old" — issue age since first seen. */
function age(firstSeen: number): string {
  return timeAgo(firstSeen).replace(' ago', ' old').replace('a few seconds old', 'new');
}

/** Renders `X` placeholders (normalised numbers) as inline code, like the issue list does. */
function Message({ text }: { text: string }) {
  const parts = text.split(/\b(X)\b/);
  return (
    <>
      {parts.map((p, i) =>
        p === 'X' ? (
          <code key={i} className="msg-placeholder">
            X
          </code>
        ) : (
          p
        ),
      )}
    </>
  );
}

interface Props {
  issue: Issue;
  selected: boolean;
  onOpen: (id: string) => void;
}

export const IssueRow = memo(function IssueRow({ issue, selected, onOpen }: Props) {
  return (
    <div className={`issue-row${selected ? ' selected' : ''}`} onClick={() => onOpen(issue.id)} role="button" tabIndex={0} onKeyDown={(e) => e.key === 'Enter' && onOpen(issue.id)}>
      <div className="issue-details">
        <div className="issue-title">
          <PlatformIcon platform={issue.platform} />
          <span>{issue.errorType}</span>
        </div>
        <div className="issue-message">
          <Message text={issue.message} />
        </div>
        <div className="issue-chips">
          <AppChip issue={issue} />
          {issue.location && (
            <>
              <code className="issue-fn">{issue.location.function}</code>
              {issue.location.file && <span className="issue-file">in {issue.location.file}</span>}
            </>
          )}
        </div>
        <div className="issue-meta">
          <span>
            Last seen {timeAgo(issue.lastSeen)} – {age(issue.firstSeen)}
          </span>
          {/* Controls inside the row must not open the panel. */}
          <span className="issue-badges" onClick={(e) => e.stopPropagation()}>
            {issue.isCrash && <CrashBadge />}
            {issue.suspectedCause && !issue.isCrash && <CauseSelect issueId={issue.id} cause={issue.suspectedCause} />}
            {issue.isRegression && <RegressionBadge />}
          </span>
        </div>
      </div>
      <div className="issue-count">
        <Sparkline values={issue.sparkline} />
        <span>{issue.count.toLocaleString()}</span>
      </div>
      <div className="issue-status" onClick={(e) => e.stopPropagation()}>
        <StatusSelect issueId={issue.id} status={issue.status} />
      </div>
      <div className="issue-assignee" onClick={(e) => e.stopPropagation()}>
        <AssigneeSelect issueId={issue.id} assignee={issue.assignee} />
      </div>
    </div>
  );
});
