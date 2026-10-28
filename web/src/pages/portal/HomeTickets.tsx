import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Siren, Ticket } from 'lucide-react';
import type { PortalTicket, TicketSource } from '../../api/portal/client';
import { age } from '../../api/portal/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { ConnectOverlay, HomeSection, Priority, ProviderTabs, type ProviderTab } from './homeCommon';
import { useTickets } from './homeQueries';

const TABS: ProviderTab<TicketSource>[] = [
  { key: 'jira', label: 'Jira', logo: 'jira' },
  { key: 'linear', label: 'Linear', logo: 'linear' },
  { key: 'work_items', label: 'Work Items', logo: <Siren size={14} /> },
];
const LABEL: Record<TicketSource, string> = { jira: 'Jira', linear: 'Linear', work_items: 'Work Items' };

const PLACEHOLDER: PortalTicket[] = [
  ['SHOP-101', 'Update the shipping rates table', 'In Progress', 'high'],
  ['SHOP-98', 'Add search to the order history', 'To Do', 'medium'],
  ['SHOP-87', 'Alert on failed refunds', 'To Do', 'urgent'],
  ['SHOP-80', 'Remove the old banner service', 'In Review', 'low'],
].map(([key, title, status, level], i) => ({
  key,
  title,
  url: '',
  status,
  statusCategory: 'todo',
  createdAt: Date.now() - (i + 2) * 86_400_000,
  updatedAt: Date.now() - (i + 1) * 3_600_000,
  priority: level[0].toUpperCase() + level.slice(1),
  priorityLevel: level,
}));

function statusClass(category: string): string {
  return category === 'done' ? 'ph-status-done' : category === 'in_progress' ? 'ph-status-progress' : 'ph-status-todo';
}

function TicketTable({ tickets }: { tickets: PortalTicket[] }) {
  return (
    <table className="ph-grid-table">
      <thead>
        <tr>
          <th style={{ width: 110 }}>Key</th>
          <th>Title</th>
          <th style={{ width: 130 }}>Status</th>
          <th style={{ width: 100 }}>Created</th>
          <th style={{ width: 100 }}>Updated</th>
          <th style={{ width: 110 }}>Priority</th>
        </tr>
      </thead>
      <tbody>
        {tickets.map((t) => (
          <tr key={t.key}>
            <td className="mono">
              {!t.url ? (
                t.key
              ) : t.url.startsWith('/') ? (
                <Link to={t.url}>{t.key}</Link>
              ) : (
                <a href={t.url} target="_blank" rel="noreferrer noopener">
                  {t.key}
                </a>
              )}
            </td>
            <td className="ph-ellipsis">{t.title}</td>
            <td>
              <span className={`ph-status ${statusClass(t.statusCategory)}`}>{t.status}</span>
            </td>
            <td className="sc-muted">{age(t.createdAt)}</td>
            <td className="sc-muted">{age(t.updatedAt)}</td>
            <td>
              <Priority level={t.priorityLevel} label={t.priority} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function SourceBody({ source }: { source: TicketSource }) {
  const navigate = useNavigate();
  const q = useTickets(source);
  if (q.isLoading) return <Loading />;
  if (q.error) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const data = q.data!;
  const label = LABEL[source];
  if (data.status !== 'ok') {
    return (
      <ConnectOverlay
        logo={source}
        name={label}
        title={data.status === 'error' ? `Could not connect to ${label}` : data.message}
        text={data.status === 'error' ? data.message : `Connect ${label} to see the work assigned to you.`}
        action={data.status === 'identity_required' ? `Set your ${label} email` : `Connect ${label}`}
        onAction={() => navigate('/portal/settings/configurations')}
      >
        <TicketTable tickets={PLACEHOLDER} />
      </ConnectOverlay>
    );
  }
  if (data.tickets.length === 0) {
    return <div className="ph-empty-line">{source === 'work_items' ? 'No open incidents assigned to you.' : `No open ${label} issues assigned to ${data.user}.`}</div>;
  }
  return <TicketTable tickets={data.tickets} />;
}

/** Home → Your Tickets: Jira and Linear issues assigned to the member, and the incidents they work on. */
export function HomeTickets() {
  const [tab, setTab] = useState<TicketSource>('jira');
  return (
    <HomeSection id="tickets" icon={<Ticket size={16} />} title="Your Tickets">
      <ProviderTabs tabs={TABS} value={tab} onChange={setTab} />
      <SourceBody key={tab} source={tab} />
    </HomeSection>
  );
}
