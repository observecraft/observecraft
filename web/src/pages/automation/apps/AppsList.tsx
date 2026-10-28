import { useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ExternalLink, LayoutGrid, Pencil, Plus, Search, Trash2 } from 'lucide-react';
import type { AutomationApp } from '../../../api/apps/types';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { useApps, useDeleteApp } from './queries';
import './apps.css';

const ago = (t: number) => {
  const m = Math.round((Date.now() - t) / 60_000);
  if (m < 1) return 'just now';
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return new Date(t).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
};

/** All Apps: the org's apps, searchable, with their status and owners. */
export function AppsList() {
  const apps = useApps();
  const remove = useDeleteApp();
  const navigate = useNavigate();
  const [query, setQuery] = useState('');
  const [status, setStatus] = useState<'all' | 'published' | 'draft'>('all');
  const [deleting, setDeleting] = useState<AutomationApp | null>(null);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (apps.data ?? []).filter(
      (a) => (status === 'all' || (status === 'published') === a.published) && (!q || `${a.name} ${a.description} ${a.tags.join(' ')} ${a.createdBy}`.toLowerCase().includes(q)),
    );
  }, [apps.data, query, status]);

  return (
    <div className="ab-list-page">
      <div className="ab-list-toolbar">
        <div className="ab-search">
          <Search size={14} />
          <input placeholder="Search apps by name, description, tag or creator" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <div className="ab-seg">
          {(['all', 'published', 'draft'] as const).map((s) => (
            <button key={s} className={status === s ? 'active' : ''} onClick={() => setStatus(s)}>
              {s === 'all' ? 'All' : s === 'published' ? 'Published' : 'Drafts'}
            </button>
          ))}
        </div>
        <Link className="ab-new-btn" to="/automation/apps/new">
          <Plus size={14} /> New App
        </Link>
      </div>
      {apps.isLoading ? (
        <Loading />
      ) : apps.isError ? (
        <ErrorState error={apps.error} onRetry={() => apps.refetch()} />
      ) : shown.length === 0 ? (
        <div className="ab-empty">
          <Search size={40} />
          <h3>{(apps.data ?? []).length === 0 ? 'No apps yet' : 'No matching results found'}</h3>
          <p>{(apps.data ?? []).length === 0 ? 'Start from a blueprint or an empty canvas.' : 'Try another search or status.'}</p>
          <div className="ab-empty-actions">
            <Link className="ab-new-btn" to="/automation/apps/new">
              <Plus size={14} /> New App
            </Link>
            <Link className="btn" to="/automation/apps/blueprints">
              Browse blueprints
            </Link>
          </div>
        </div>
      ) : (
        <>
          <div className="ab-count">
            {shown.length} app{shown.length === 1 ? '' : 's'}
          </div>
          <table className="ab-list">
            <thead>
              <tr>
                <th>Name</th>
                <th>Description</th>
                <th>Status</th>
                <th>Created by</th>
                <th>Last modified</th>
                <th>Tags</th>
                <th aria-label="Actions" />
              </tr>
            </thead>
            <tbody>
              {shown.map((a) => (
                <tr key={a.id} onClick={() => navigate(`/automation/apps/${a.id}`)}>
                  <td>
                    <span className="ab-list-name">
                      <span className="ab-app-icon sm">
                        <LayoutGrid size={13} />
                      </span>
                      {a.name}
                    </span>
                  </td>
                  <td className="ab-list-desc">{a.description}</td>
                  <td>
                    <span className={`ab-chip ${a.published ? 'ab-chip-published' : 'ab-chip-draft'}`}>{a.published ? 'Published' : 'Draft'}</span>
                  </td>
                  <td>{a.createdBy || '—'}</td>
                  <td className="ab-muted">{ago(a.updatedAt)}</td>
                  <td>
                    <span className="ab-tags">
                      {a.tags.map((t) => (
                        <span key={t} className="ab-tag">
                          {t}
                        </span>
                      ))}
                    </span>
                  </td>
                  <td className="ab-row-actions" onClick={(e) => e.stopPropagation()}>
                    <Link className="ab-icon-btn" to={`/automation/apps/${a.id}`} aria-label={`Open ${a.name}`} title="Open">
                      <ExternalLink size={14} />
                    </Link>
                    <Link className="ab-icon-btn" to={`/automation/apps/${a.id}/edit`} aria-label={`Edit ${a.name}`} title="Edit">
                      <Pencil size={14} />
                    </Link>
                    <button className="ab-icon-btn" aria-label={`Delete ${a.name}`} title="Delete" onClick={() => setDeleting(a)}>
                      <Trash2 size={14} />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
      {deleting && (
        <Modal label="Delete app" onClose={() => setDeleting(null)} width={440} height="auto">
          <div className="ab-confirm">
            <h3>Delete {deleting.name}?</h3>
            <p>The app goes away for everyone. The workflows its buttons run are kept.</p>
            {remove.isError && <div className="ab-alert">{remove.error instanceof Error ? remove.error.message : 'The app could not be deleted'}</div>}
            <div className="ab-confirm-actions">
              <button className="btn" onClick={() => setDeleting(null)}>
                Cancel
              </button>
              <button className="btn ab-danger" disabled={remove.isPending} onClick={() => remove.mutate(deleting.id, { onSuccess: () => setDeleting(null) })}>
                Delete
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}
