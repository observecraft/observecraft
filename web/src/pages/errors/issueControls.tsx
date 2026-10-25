import { ChevronDown } from 'lucide-react';
import type { IssueStatus, SuspectedCause, User } from '../../api/errorTracking/types';
import { Select, type Option } from '../../components/ui/Dropdown';
import { useUpdateIssue, useUsers } from './queries';
import { Avatar, CAUSE_LABEL, STATUS_LABEL } from './issueVisuals';

const STATUS_OPTIONS: Option<IssueStatus>[] = (Object.keys(STATUS_LABEL) as IssueStatus[]).map((s) => ({
  value: s,
  label: <span className={`status-text status-${s}`}>{STATUS_LABEL[s].toUpperCase()}</span>,
}));

/** "FOR REVIEW ▾" — changes the issue's workflow state. */
export function StatusSelect({ issueId, status }: { issueId: string; status: IssueStatus }) {
  const update = useUpdateIssue();
  return (
    <Select
      value={status}
      options={STATUS_OPTIONS}
      onChange={(s) => update.mutate({ id: issueId, patch: { status: s } })}
      triggerClassName={`status-trigger status-${status}`}
      trigger={() => (
        <>
          {STATUS_LABEL[status].toUpperCase()}
          <ChevronDown size={14} />
        </>
      )}
    />
  );
}

const UNASSIGNED = '__none__';

export function AssigneeSelect({ issueId, assignee, label }: { issueId: string; assignee: User | null; label?: string }) {
  const update = useUpdateIssue();
  const users = useUsers();
  const options: Option<string>[] = [
    { value: UNASSIGNED, label: 'Unassigned', icon: <Avatar user={null} size={20} /> },
    ...(users.data ?? []).map((u) => ({ value: u.id, label: u.name, icon: <Avatar user={u} size={20} /> })),
  ];
  return (
    <Select
      value={assignee?.id ?? UNASSIGNED}
      options={options}
      align="right"
      title={assignee ? `Assigned to ${assignee.name}` : 'Assign'}
      onChange={(v) => update.mutate({ id: issueId, patch: { assigneeId: v === UNASSIGNED ? null : v } })}
      triggerClassName="assignee-trigger"
      trigger={() => (
        <>
          <Avatar user={assignee} size={label ? 18 : 26} />
          {label && <span>{assignee?.name ?? label}</span>}
          <ChevronDown size={13} className="dd-caret" />
        </>
      )}
    />
  );
}

const CAUSE_OPTIONS: Option<SuspectedCause>[] = (Object.keys(CAUSE_LABEL) as SuspectedCause[]).map((c) => ({ value: c, label: CAUSE_LABEL[c] }));

/** Orange "Code exception ▾" pill — lets a reviewer correct the automatic classification. */
export function CauseSelect({ issueId, cause }: { issueId: string; cause: SuspectedCause }) {
  const update = useUpdateIssue();
  return (
    <Select
      value={cause}
      options={CAUSE_OPTIONS}
      onChange={(c) => update.mutate({ id: issueId, patch: { suspectedCause: c } })}
      triggerClassName="cause-trigger"
      trigger={() => (
        <>
          {CAUSE_LABEL[cause]}
          <ChevronDown size={13} />
        </>
      )}
    />
  );
}
