import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { ExternalLink, Plus, RadioTower, Search } from 'lucide-react';
import { VISIBILITY_LABEL, type StatusPageSummary } from '../../api/statusPages/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatLongDate, timeAgo } from '../../lib/format';
import { useProfile } from '../settings/queries';
import { useStatusPages } from './queries';
import { ComponentStatusLabel } from './StatusParts';
import '../settings/settings.css';
import '../logs/logs.css';

/** Standard users and admins create and edit status pages and post notices. */
export function useCanEditStatusPages(): boolean {
  const me = useProfile();
  const org = me.data?.org.id;
  return me.data?.roles.some((r) => r.id === `${org}-admin` || r.id === `${org}-standard`) ?? false;
}

const matches = (p: StatusPageSummary, search: string) => {
  const s = search.trim().toLowerCase();
  return !s || p.name.toLowerCase().includes(s);
};

/** Incident Response → Status Pages: the org's pages and their current status. */
export function StatusPagesPage() {
  const q = useStatusPages();
  const canEdit = useCanEditStatusPages();
  const [search, setSearch] = useState('');
  const shown = useMemo(() => (q.data ?? []).filter((p) => matches(p, search)), [q.data, search]);

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const create = canEdit && (
    <Link className="btn btn-primary" to="/status-pages/new">
      <Plus size={14} /> Create Status Page
    </Link>
  );
  return (
    <div className="stp-page">
      <div className="stp-hero">
        <div>
          <h1 className="settings-title">Status Pages</h1>
          <p className="muted">
            Keep your users informed: show the components they rely on with 60 days of uptime, and post notices while an incident affects them. Public pages are open to anyone with the link; internal
            pages only to members of this organization.
          </p>
        </div>
        {q.data.length > 0 && create}
      </div>
      <section className="stp-card">
        <div className="stp-card-head">
          <h2>Status Pages</h2>
          <label className="settings-search small stp-filter">
            <Search size={14} />
            <input placeholder="Filter by page name" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Filter by page name" />
          </label>
        </div>
        {q.data.length === 0 ? (
          <div className="stp-empty">
            <RadioTower size={36} />
            <p className="muted">No status pages created yet.</p>
            {create}
          </div>
        ) : shown.length === 0 ? (
          <p className="muted stp-card-empty">No status page matches the filter.</p>
        ) : (
          <table className="settings-table ix-table">
            <thead>
              <tr>
                <th>NAME</th>
                <th>PAGE</th>
                <th>VISIBILITY</th>
                <th className="num">COMPONENTS</th>
                <th>STATUS</th>
                <th className="num">OPEN NOTICES</th>
                <th>LAST UPDATED</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((p) => (
                <tr key={p.id}>
                  <td>
                    <Link to={`/status-pages/${p.id}`}>{p.name}</Link>
                  </td>
                  <td>
                    <a className="mono" href={`/status/${p.slug}`} target="_blank" rel="noreferrer">
                      /status/{p.slug} <ExternalLink size={12} />
                    </a>
                  </td>
                  <td>
                    <span className="tag-chip">{VISIBILITY_LABEL[p.visibility]}</span>
                  </td>
                  <td className="num">{p.componentCount}</td>
                  <td>
                    <ComponentStatusLabel status={p.overallStatus} />
                  </td>
                  <td className="num">{p.openNotices || <span className="muted">—</span>}</td>
                  <td title={formatLongDate(p.updatedAt)}>
                    {timeAgo(p.updatedAt)}
                    {p.updatedBy && <span className="muted"> by {p.updatedBy}</span>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
