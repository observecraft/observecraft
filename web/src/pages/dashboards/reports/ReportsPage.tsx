import { useCallback, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { Bell, EllipsisVertical, Info, List, Plus, Search, Send, User } from 'lucide-react';
import { TIME_FRAMES, type DashboardReport, type DashboardReportDelivery } from '../../../api/dashboardReports/types';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Menu } from '../../../components/ui/Dropdown';
import { Modal } from '../../../components/ui/Modal';
import { timeAgo } from '../../../lib/format';
import { DashboardPickerModal } from '../DashboardPickerModal';
import { DashboardsHeader } from '../list/DashboardsHeader';
import { Initials } from '../list/DashboardListPage';
import { useDeleteReport, useMe, useReportOptions, useReportPreview, useReports, useSendReport, useSubscribeReport } from './queries';
import { ReportEditorModal } from './ReportEditorModal';
import '../dashboards.css';
import './reports.css';

type View = 'mine' | 'all' | 'subscribed';

const VIEWS: { key: View; label: string; title: string; icon: typeof List }[] = [
  { key: 'mine', label: 'Created by you', title: 'Your Reports', icon: User },
  { key: 'all', label: 'All reports', title: 'All Reports', icon: List },
  { key: 'subscribed', label: 'Subscribed', title: 'Subscribed Reports', icon: Bell },
];

const STATUS: Record<string, string> = { sent: 'Sent', partial: 'Partly sent', failed: 'Failed' };

export function ReportsPage() {
  const [params, setParams] = useSearchParams();
  const view = (params.get('view') as View | null) ?? 'mine';
  const query = params.get('q') ?? '';
  const set = useCallback(
    (k: string, v: string | null) =>
      setParams(
        (p) => {
          const n = new URLSearchParams(p);
          if (v) n.set(k, v);
          else n.delete(k);
          return n;
        },
        { replace: true },
      ),
    [setParams],
  );
  const reports = useReports();
  const options = useReportOptions();
  const me = useMe();
  const subscribe = useSubscribeReport();
  const del = useDeleteReport();
  const [picking, setPicking] = useState(false);
  const [editing, setEditing] = useState<{ dashboardId: string; report?: DashboardReport } | null>(null);
  const [previewing, setPreviewing] = useState<DashboardReport | null>(null);

  const myId = me.data?.id;
  const q = query.trim().toLowerCase();
  const rows = (reports.data ?? [])
    .filter((r) => (view === 'mine' ? r.author?.id === myId : view === 'subscribed' ? r.subscribed : true))
    .filter((r) => !q || r.name.toLowerCase().includes(q) || r.dashboardTitle.toLowerCase().includes(q));
  const current = VIEWS.find((v) => v.key === view) ?? VIEWS[0];

  return (
    <div className="dl-page">
      <DashboardsHeader>
        <button className="btn btn-primary" onClick={() => setPicking(true)}>
          <Plus size={14} /> New Report
        </button>
      </DashboardsHeader>
      <div className="dl-scroll">
        <label className="int-search dl-search">
          <Search size={16} />
          <input placeholder="Search reports by name" value={query} onChange={(e) => set('q', e.target.value || null)} />
        </label>
        <div className="dl-body">
          <aside className="dl-side">
            {VIEWS.map((v) => (
              <button key={v.key} className={`dl-list${view === v.key ? ' active' : ''}`} onClick={() => set('view', v.key === 'mine' ? null : v.key)}>
                <v.icon size={15} /> <span className="dl-list-name">{v.label}</span>
              </button>
            ))}
          </aside>
          <main className="dl-main">
            <h2 className="dl-h2">{current.title}</h2>
            {options.data && !options.data.emailEnabled && (
              <p className="rp-note rp-banner">
                <Info size={14} /> Email isn't configured on this server (OC_SMTP_URL). Reports reach their notification channels; email recipients and subscribers are skipped until it is.
              </p>
            )}
            {reports.isPending ? (
              <Loading label="Loading reports…" />
            ) : reports.isError ? (
              <ErrorState error={reports.error} onRetry={() => reports.refetch()} />
            ) : rows.length === 0 ? (
              q ? (
                <Empty>No report matches “{query}”.</Empty>
              ) : (
                <div className="dash-empty rp-empty">
                  <List size={40} />
                  <h3>No Reports</h3>
                  <p>To create one, add a report to any dashboard</p>
                  <button className="btn btn-primary" onClick={() => setPicking(true)}>
                    <Plus size={14} /> New Report
                  </button>
                </div>
              )
            ) : (
              <div className="dl-table-card rp-table-card">
                <table className="dl-table rp-table">
                  <thead>
                    <tr>
                      <th>REPORT NAME</th>
                      <th>DASHBOARD</th>
                      <th>SCHEDULE</th>
                      <th>RECIPIENTS</th>
                      <th>LAST SENT</th>
                      <th>AUTHOR</th>
                      <th title="Receive this report by email">SUBSCRIBE</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((r) => (
                      <tr key={r.id} onClick={() => setEditing({ dashboardId: r.dashboardId, report: r })}>
                        <td className="dl-name">{r.name}</td>
                        <td onClick={(e) => e.stopPropagation()}>
                          <Link to={`/dashboards/${r.dashboardId}`}>{r.dashboardTitle}</Link>
                        </td>
                        <td>
                          <div>{r.scheduleLabel}</div>
                          <div className="muted-text">{TIME_FRAMES.find((t) => t.key === r.timeFrame)?.label ?? r.timeFrame}</div>
                        </td>
                        <td className="rp-recips">
                          {r.recipients.slice(0, 3).map((x) => (
                            <span key={x} className="tag-chip">
                              {x.includes('@') ? x : `@${x}`}
                            </span>
                          ))}
                          {r.recipients.length > 3 && <span className="muted-text"> +{r.recipients.length - 3}</span>}
                          {r.subscriberCount > 0 && <span className="muted-text"> · {r.subscriberCount} subscribed</span>}
                        </td>
                        <td>
                          {r.lastSentAt ? (
                            <span title={r.lastError ?? undefined}>
                              {timeAgo(r.lastSentAt)} <span className={`rp-status rp-${r.lastStatus}`}>{STATUS[r.lastStatus ?? ''] ?? r.lastStatus}</span>
                            </span>
                          ) : (
                            <span className="muted-text">Never</span>
                          )}
                        </td>
                        <td>
                          {r.author && (
                            <span className="dl-author">
                              <Initials name={r.author.name} /> {r.author.name.toUpperCase()}
                            </span>
                          )}
                        </td>
                        <td onClick={(e) => e.stopPropagation()}>
                          <button
                            role="switch"
                            aria-checked={r.subscribed}
                            aria-label={`Subscribe to ${r.name}`}
                            className={`switch${r.subscribed ? ' on' : ''}`}
                            onClick={() => subscribe.mutate({ id: r.id, subscribed: !r.subscribed })}
                          />
                        </td>
                        <td onClick={(e) => e.stopPropagation()}>
                          <Menu
                            triggerClassName="icon-plain"
                            trigger={<EllipsisVertical size={16} />}
                            items={[
                              { label: 'Edit', onClick: () => setEditing({ dashboardId: r.dashboardId, report: r }) },
                              { label: 'Preview and send now', onClick: () => setPreviewing(r) },
                              { label: 'Delete', onClick: () => del.mutate(r.id) },
                            ]}
                          />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </main>
        </div>
      </div>
      {picking && (
        <DashboardPickerModal
          title="New report: choose a dashboard"
          onClose={() => setPicking(false)}
          onPick={(d) => {
            setPicking(false);
            setEditing({ dashboardId: d.id });
          }}
        />
      )}
      {editing && <ReportEditorModal dashboardId={editing.dashboardId} report={editing.report} onClose={() => setEditing(null)} />}
      {previewing && <ReportPreviewModal report={previewing} onClose={() => setPreviewing(null)} />}
    </div>
  );
}

/** The message the report's recipients receive, and "Send now". */
export function ReportPreviewModal({ report, onClose }: { report: DashboardReport; onClose: () => void }) {
  const preview = useReportPreview(report.id);
  const options = useReportOptions();
  const send = useSendReport();
  const [sent, setSent] = useState<DashboardReportDelivery[] | null>(null);
  return (
    <Modal onClose={onClose} label="Report preview" width={720} height="auto">
      <div className="small-modal">
        <h3>{report.name}</h3>
        {preview.isPending ? (
          <Loading label="Summarizing the dashboard…" />
        ) : preview.isError ? (
          <ErrorState error={preview.error} onRetry={() => preview.refetch()} />
        ) : (
          <>
            <div className="muted-text">Subject: {preview.data.subject}</div>
            <pre className="rp-body">{preview.data.body}</pre>
          </>
        )}
        {options.data && !options.data.emailEnabled && (
          <p className="rp-note">
            <Info size={14} /> Email isn't configured on this server: Send now delivers to the report's channels only.
          </p>
        )}
        {sent && (
          <ul className="rp-deliveries">
            {sent.length === 0 && <li className="muted-text">The report has no recipients.</li>}
            {sent.map((d) => (
              <li key={d.recipient}>
                <span className={`rp-status rp-${d.status === 'sent' ? 'sent' : d.status === 'failed' ? 'failed' : 'partial'}`}>
                  {d.status === 'sent' ? 'Sent' : d.status === 'failed' ? 'Failed' : 'Not sent'}
                </span>{' '}
                {d.recipient.includes('@') ? d.recipient : `@${d.recipient}`}
                {d.error && <span className="muted-text"> — {d.error}</span>}
              </li>
            ))}
          </ul>
        )}
        {send.error && <p className="form-error">{send.error.message}</p>}
        <div className="small-modal-actions">
          <button className="btn" onClick={onClose}>
            Close
          </button>
          <button className="btn btn-primary" disabled={send.isPending} onClick={() => send.mutate(report.id, { onSuccess: setSent })}>
            <Send size={14} /> {send.isPending ? 'Sending…' : 'Send now'}
          </button>
        </div>
      </div>
    </Modal>
  );
}
