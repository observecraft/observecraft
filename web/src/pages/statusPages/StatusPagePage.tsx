import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { ArrowLeft, ExternalLink, MessageSquarePlus, Pencil, Plus, Trash2 } from 'lucide-react';
import type { StatusPageNotice } from '../../api/statusPages/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatLongDate, timeAgo } from '../../lib/format';
import { NoticeModal } from './NoticeModal';
import { useStatusPage, useStatusPageMutations } from './queries';
import { VISIBILITY_LABEL } from '../../api/statusPages/types';
import { useCanEditStatusPages } from './StatusPagesPage';
import { ComponentList, NoticeCard, OverallBanner } from './StatusParts';
import '../settings/settings.css';
import '../logs/logs.css';

/** Incident Response → Status Pages → a page: its components with uptime, and its notices. */
export function StatusPagePage() {
  const { id = '' } = useParams();
  const navigate = useNavigate();
  const q = useStatusPage(id);
  const canEdit = useCanEditStatusPages();
  const m = useStatusPageMutations();
  const [noticeFor, setNoticeFor] = useState<StatusPageNotice | 'new' | null>(null);

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const page = q.data;
  const open = page.notices.filter((n) => n.resolvedAt == null);
  const past = page.notices.filter((n) => n.resolvedAt != null);
  const confirmDelete = () =>
    window.confirm(`Delete the status page "${page.name}" and its notices? /status/${page.slug} stops working. This can't be undone.`) &&
    m.remove.mutate(page.id, { onSuccess: () => navigate('/status-pages') });
  const deleteNotice = (n: StatusPageNotice) =>
    window.confirm(`Delete the notice "${n.title}" and its updates? It no longer counts in the uptime history.`) && m.removeNotice.mutate({ id: page.id, noticeId: n.id });
  const noticeActions = (n: StatusPageNotice) =>
    canEdit && (
      <div className="stp-actions">
        {n.resolvedAt == null && (
          <button className="btn btn-sm" onClick={() => setNoticeFor(n)}>
            <MessageSquarePlus size={14} /> Post Update
          </button>
        )}
        <button className="icon-plain" title="Delete notice" onClick={() => deleteNotice(n)}>
          <Trash2 size={14} />
        </button>
      </div>
    );
  const noticeMutation = noticeFor === 'new' ? m.createNotice : m.updateNotice;

  return (
    <div className="stp-page">
      <Link to="/status-pages" className="stp-back">
        <ArrowLeft size={14} /> Status Pages
      </Link>
      <div className="settings-title-row">
        <h1 className="settings-title">{page.name}</h1>
        <div className="pl-title-actions">
          <a className="btn" href={`/status/${page.slug}`} target="_blank" rel="noreferrer">
            <ExternalLink size={14} /> View Page
          </a>
          {canEdit && (
            <>
              <Link className="btn" to={`/status-pages/${page.id}/edit`}>
                <Pencil size={14} /> Edit
              </Link>
              <button className="btn" onClick={confirmDelete} disabled={m.remove.isPending}>
                <Trash2 size={14} /> Delete
              </button>
            </>
          )}
        </div>
      </div>
      {page.description && <p>{page.description}</p>}
      <dl className="stp-details">
        <div>
          <dt>Page</dt>
          <dd className="mono">/status/{page.slug}</dd>
        </div>
        <div>
          <dt>Visibility</dt>
          <dd>
            {VISIBILITY_LABEL[page.visibility]}
            <span className="muted"> · {page.visibility === 'public' ? 'anyone with the link' : 'members of this organization'}</span>
          </dd>
        </div>
        <div>
          <dt>Last updated</dt>
          <dd title={formatLongDate(page.updatedAt)}>
            {timeAgo(page.updatedAt)}
            {page.updatedBy && <span className="muted"> by {page.updatedBy}</span>}
          </dd>
        </div>
        <div>
          <dt>Created</dt>
          <dd title={formatLongDate(page.createdAt)}>
            {timeAgo(page.createdAt)}
            {page.createdBy && <span className="muted"> by {page.createdBy}</span>}
          </dd>
        </div>
      </dl>
      {(m.remove.error || m.removeNotice.error) && <p className="form-error">{((m.remove.error || m.removeNotice.error) as Error).message}</p>}
      <OverallBanner status={page.overallStatus} />

      <div className="stp-section-head">
        <h2 className="settings-subtitle">Open Notices</h2>
        {canEdit && (
          <button className="btn btn-primary" onClick={() => setNoticeFor('new')}>
            <Plus size={14} /> New Notice
          </button>
        )}
      </div>
      {open.length ? (
        <div className="stp-notices">
          {open.map((n) => (
            <NoticeCard key={n.id} notice={n} components={page.components} actions={noticeActions(n)} />
          ))}
        </div>
      ) : (
        <p className="muted">No open notices.</p>
      )}

      <h2 className="settings-subtitle">
        Components <span className="muted">{page.components.length}</span>
      </h2>
      <p className="muted">One bar per day of the last 60 (UTC), from the notices&apos; history; partial and major outages count as downtime.</p>
      <ComponentList components={page.components} notices={page.notices} visualization={page.visualization} />

      <h2 className="settings-subtitle">Past Notices</h2>
      {past.length ? (
        <div className="stp-notices">
          {past.map((n) => (
            <NoticeCard key={n.id} notice={n} components={page.components} actions={noticeActions(n)} />
          ))}
        </div>
      ) : (
        <p className="muted">No notices in the last 60 days.</p>
      )}

      {noticeFor && (
        <NoticeModal
          components={page.components}
          notice={noticeFor === 'new' ? undefined : noticeFor}
          pending={noticeMutation.isPending}
          error={noticeMutation.error}
          onClose={() => setNoticeFor(null)}
          onCreate={(input) => m.createNotice.mutate({ id: page.id, input }, { onSuccess: () => setNoticeFor(null) })}
          onUpdate={(input) => noticeFor !== 'new' && m.updateNotice.mutate({ id: page.id, noticeId: noticeFor.id, input }, { onSuccess: () => setNoticeFor(null) })}
        />
      )}
    </div>
  );
}
