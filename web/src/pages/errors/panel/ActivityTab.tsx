import type { ReactNode } from 'react';
import { CircleDot, MessageSquare, RefreshCcw, Sparkle, UserPlus } from 'lucide-react';
import type { ActivityEntry, IssueDetail } from '../../../api/errorTracking/types';
import { formatLongDate, timeAgo } from '../../../lib/format';
import { STATUS_LABEL } from '../issueVisuals';

function describe(a: ActivityEntry): { icon: ReactNode; text: ReactNode } {
  switch (a.kind) {
    case 'created':
      return { icon: <Sparkle size={14} />, text: 'Issue first seen' };
    case 'regression':
      return { icon: <RefreshCcw size={14} />, text: <>Regression detected{a.version && <> in version <code>{a.version}</code></>}</> };
    case 'status':
      return {
        icon: <CircleDot size={14} />,
        text: (
          <>
            <strong>{a.by?.name ?? 'System'}</strong> changed status from <strong>{STATUS_LABEL[a.from]}</strong> to <strong>{STATUS_LABEL[a.to]}</strong>
          </>
        ),
      };
    case 'assign':
      return {
        icon: <UserPlus size={14} />,
        text: (
          <>
            <strong>{a.by?.name ?? 'System'}</strong> {a.to ? <>assigned <strong>{a.to.name}</strong></> : 'removed the assignee'}
          </>
        ),
      };
    case 'comment':
      return {
        icon: <MessageSquare size={14} />,
        text: (
          <>
            <strong>{a.by.name}</strong> commented: “{a.text}”
          </>
        ),
      };
  }
}

export function ActivityTab({ issue }: { issue: IssueDetail }) {
  return (
    <ol className="activity">
      {issue.activity.map((a, i) => {
        const d = describe(a);
        return (
          <li key={i}>
            <span className="activity-icon">{d.icon}</span>
            <span className="activity-text">{d.text}</span>
            <span className="muted-text" title={formatLongDate(a.at)}>
              {timeAgo(a.at)}
            </span>
          </li>
        );
      })}
    </ol>
  );
}
