import { useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Plus, Search, Table2 } from 'lucide-react';
import type { ReferenceTable } from '../../api/referenceTables/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatLongDate, timeAgo } from '../../lib/format';
import { useProfile } from '../settings/queries';
import { useReferenceTables } from './queries';
import '../settings/settings.css';
import '../logs/logs.css';

/** Standard users and admins create, replace and delete reference tables. */
export function useCanEditReferenceTables(): boolean {
  const me = useProfile();
  const org = me.data?.org.id;
  return me.data?.roles.some((r) => r.id === `${org}-admin` || r.id === `${org}-standard`) ?? false;
}

export const SOURCE_LABELS: Record<string, string> = { local_file: 'Local file' };

const matches = (t: ReferenceTable, search: string) => {
  const s = search.trim().toLowerCase();
  return !s || [t.name, t.description, t.primaryKey, t.createdBy ?? ''].some((v) => v.toLowerCase().includes(s));
};

/** Integrations → Reference Tables: the org's uploaded lookup tables. */
export function ReferenceTablesPage() {
  const q = useReferenceTables();
  const canEdit = useCanEditReferenceTables();
  const [search, setSearch] = useState('');
  const navigate = useNavigate();
  const shown = useMemo(() => (q.data ?? []).filter((t) => matches(t, search)), [q.data, search]);

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (q.data.length === 0)
    return (
      <div className="int-page">
        <div className="rt-empty">
          <Table2 size={56} strokeWidth={1.25} className="brand-icon" />
          <h2>Get started with Reference Tables</h2>
          <p className="muted">Upload a CSV of your own data, such as the team owning each service or the name of each SKU, and look its rows up by a primary key.</p>
          {canEdit && (
            <button className="btn btn-primary" onClick={() => navigate('/integrations/reference-tables/new')}>
              <Plus size={14} /> Create Your First Table
            </button>
          )}
        </div>
      </div>
    );
  return (
    <div className="int-page">
      <div className="settings-title-row">
        <h1 className="settings-title">Reference Tables</h1>
        {canEdit && (
          <button className="btn btn-primary" onClick={() => navigate('/integrations/reference-tables/new')}>
            <Plus size={14} /> New Reference Table
          </button>
        )}
      </div>
      <p className="muted">
        Tables of your own data, uploaded as CSV files, with a primary key column to look rows up by: the team owning each service, the product name and category of each SKU, the region of each
        account. Uploading a new file replaces every row.
      </p>
      <label className="settings-search wide">
        <Search size={16} />
        <input placeholder="Search tables by name, description, primary key or creator" value={search} onChange={(e) => setSearch(e.target.value)} />
      </label>
      {shown.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No reference table matches the search.</p>
        </section>
      ) : (
        <table className="settings-table ix-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>DESCRIPTION</th>
              <th>SOURCE</th>
              <th className="num">ROWS</th>
              <th>PRIMARY KEY</th>
              <th>LAST UPDATED</th>
              <th>CREATED BY</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((t) => (
              <tr key={t.name}>
                <td>
                  <Link className="mono" to={`/integrations/reference-tables/${encodeURIComponent(t.name)}`}>
                    {t.name}
                  </Link>
                </td>
                <td>{t.description || <span className="muted">—</span>}</td>
                <td>{SOURCE_LABELS[t.source] ?? t.source}</td>
                <td className="num">{t.rowCount.toLocaleString()}</td>
                <td>
                  <code>{t.primaryKey}</code>
                </td>
                <td title={formatLongDate(t.updatedAt)}>{timeAgo(t.updatedAt)}</td>
                <td>{t.createdBy ?? <span className="muted">—</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
