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
    <div className="stp-list-page">
      <header className="product-header">
        <div className="product-title">
          <RadioTower size={20} /> Status Pages
        </div>
      </header>
      <div className="stp-list-body">
        <h1 className="stp-list-hero">Keep your customers informed about system status</h1>
        <section className="stp-list-card">
          <div className="stp-list-card-head">
            <h2>Status Pages</h2>
            <div className="stp-list-tools">
              <label className="settings-search small stp-filter">
                <Search size={14} />
                <input placeholder="Filter by page name" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Filter by page name" />
              </label>
              {q.data.length > 0 && create}
            </div>
          </div>
          {q.data.length === 0 ? (
            <div className="stp-list-empty">
              <RadioTower size={64} strokeWidth={1.5} />
              <p>No status pages have been created yet</p>
              {create}
            </div>
          ) : shown.length === 0 ? (
            <div className="stp-list-empty">
              <p>No status page matches the filter</p>
            </div>
          ) : (
            <table className="settings-table ix-table stp-list-table">
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
    </div>
  );
}
