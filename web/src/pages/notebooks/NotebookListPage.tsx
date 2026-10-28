import { Fragment, useCallback, useDeferredValue, useEffect, useRef } from 'react';
import { Link, NavLink, useNavigate, useSearchParams } from 'react-router-dom';
import { BookOpen, Clock, EllipsisVertical, LayoutTemplate, Plus, Search, Star, Trash2, User, Users } from 'lucide-react';
import type { NotebookListView, NotebookSortField, NotebookSummary } from '../../api/notebooks/types';
import { NOTEBOOK_TEMPLATES, fromTemplate, type NotebookTemplate } from '../../api/notebooks/templates';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select } from '../../components/ui/Dropdown';
import { Pager } from '../../components/ui/Pager';
import { timeAgo } from '../../lib/format';
import { Initials } from '../dashboards/list/DashboardListPage';
import { useCloneNotebook, useCreateNotebook, useDeleteNotebook, useNotebookFavorite, useNotebookSearch, useRestoreNotebook } from './queries';
import '../dashboards/dashboards.css';
import './notebooks.css';

const PAGE = 50;

const VIEWS: { key: NotebookListView; label: string; icon: typeof Star }[] = [
  { key: 'mine', label: 'My Notebooks', icon: User },
  { key: 'team', label: "My Team's Notebooks", icon: Users },
  { key: 'recents', label: 'Recents', icon: Clock },
  { key: 'favorites', label: 'Favorites', icon: Star },
  { key: 'all', label: 'All Notebooks', icon: BookOpen },
  { key: 'deleted', label: 'Recently Deleted', icon: Trash2 },
];

/** Product header shared by the list, Templates and Reports tabs. */
function NotebooksHeader() {
  const create = useCreateNotebook();
  const navigate = useNavigate();
  const [params] = useSearchParams();
  // ?new=1 (the menus' "New Notebook") creates a notebook once and opens it.
  const wantsNew = params.get('new') === '1';
  const started = useRef(false);
  const { mutate } = create;
  useEffect(() => {
    if (!wantsNew || started.current) return;
    started.current = true;
    mutate({ title: 'Untitled notebook' }, { onSuccess: (nb) => navigate(`/notebooks/${nb.id}`, { replace: true }) });
  }, [wantsNew, mutate, navigate]);
  const tab = (to: string, label: string) => (
    <NavLink to={to} end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
      {label}
    </NavLink>
  );
  return (
    <header className="product-header">
      <div className="product-title">
        <BookOpen size={20} /> Notebooks
      </div>
      <nav className="product-tabs">
        {tab('/notebooks', 'All Notebooks')}
        {tab('/notebooks/templates', 'Templates')}
        {tab('/notebooks/reports', 'Reports')}
      </nav>
      <div className="dl-header-actions">
        <button className="btn btn-primary" disabled={create.isPending} onClick={() => create.mutate({ title: 'Untitled notebook' }, { onSuccess: (nb) => navigate(`/notebooks/${nb.id}`) })}>
          <Plus size={14} /> New Notebook
        </button>
      </div>
    </header>
  );
}

/** Creates a notebook from a template and opens it. */
function useUseTemplate() {
  const create = useCreateNotebook();
  const navigate = useNavigate();
  return { pending: create.isPending, use: (t: NotebookTemplate) => create.mutate(fromTemplate(t), { onSuccess: (nb) => navigate(`/notebooks/${nb.id}`) }) };
}

function TemplateCard({ template, onUse, detailed }: { template: NotebookTemplate; onUse: () => void; detailed?: boolean }) {
  return (
    <button className="nb-template" onClick={onUse}>
      <TemplatePreview template={template} />
      <span className="nb-template-name">{template.name}</span>
      <span className="nb-template-desc">{template.description}</span>
      {detailed && (
        <ul className="nb-template-outline">
          {template.cells.slice(0, 6).map((c, i) => (
            <li key={i}>{c.type === 'markdown' ? c.text.split('\n')[0].replace(/^#+\s*/, '') || 'Text' : c.title || c.type}</li>
          ))}
        </ul>
      )}
    </button>
  );
}

/** A sketch of the template's first cells: headings and grey lines for text, a small chart for graphs. */
function TemplatePreview({ template }: { template: NotebookTemplate }) {
  return (
    <span className="nb-template-preview" aria-hidden>
      <span className="nb-template-page">
        {template.cells.slice(0, 3).map((c, i) =>
          c.type === 'markdown' ? (
            <span key={i} className="nb-preview-text">
              <b>{c.text.split('\n')[0].replace(/^#+\s*/, '') || 'Notes'}</b>
              <i style={{ width: '88%' }} />
              <i style={{ width: '64%' }} />
            </span>
          ) : (
            <span key={i} className="nb-preview-chart">
              <b>{c.title || 'Graph'}</b>
              <svg viewBox="0 0 100 24" preserveAspectRatio="none">
                <polyline points="0,18 12,15 24,17 36,9 48,12 60,6 72,10 84,4 100,8" />
              </svg>
            </span>
          ),
        )}
      </span>
    </span>
  );
}

function FavoriteButton({ nb }: { nb: NotebookSummary }) {
  const favorite = useNotebookFavorite();
  return (
    <button
      className={`icon-plain dl-star${nb.favorite ? ' on' : ''}`}
      title={nb.favorite ? 'Remove from favorites' : 'Add to favorites'}
      onClick={(e) => (e.stopPropagation(), favorite.mutate({ id: nb.id, favorite: !nb.favorite }))}
    >
      <Star size={17} fill={nb.favorite ? 'currentColor' : 'none'} />
    </button>
  );
}

export function NotebookListPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const view = (params.get('view') as NotebookListView | null) ?? 'mine';
  const query = params.get('q') ?? '';
  const author = params.get('author');
  const team = params.get('team');
  const [sort, dir] = (params.get('sort') ?? 'modified:desc').split(':') as [NotebookSortField, 'asc' | 'desc'];
  const offset = Number(params.get('offset') ?? 0);
  const set = useCallback(
    (u: Record<string, string | null>) =>
      setParams(
        (p) => {
          const n = new URLSearchParams(p);
          for (const [k, v] of Object.entries(u)) {
            if (v === null || v === '') n.delete(k);
            else n.set(k, v);
          }
          return n;
        },
        { replace: true },
      ),
    [setParams],
  );

  const search = useNotebookSearch({ query: useDeferredValue(query), view, author, team, sort, dir, offset, limit: PAGE });
  const clone = useCloneNotebook();
  const del = useDeleteNotebook();
  const restore = useRestoreNotebook();
  const template = useUseTemplate();
  const rows = search.data?.notebooks ?? [];
  const total = search.data?.total ?? 0;

  const sortBy = (field: NotebookSortField) => set({ sort: `${field}:${field === sort && dir === 'desc' ? 'asc' : 'desc'}`, offset: null });
  const th = (label: string, field?: NotebookSortField) => (
    <th className={field && view !== 'recents' ? 'sortable' : undefined} onClick={field && view !== 'recents' ? () => sortBy(field) : undefined}>
      {label}
      {view !== 'recents' && field === sort && (dir === 'desc' ? ' ↓' : ' ↑')}
    </th>
  );

  return (
    <div className="dl-page">
      <NotebooksHeader />
      <div className="dl-scroll">
        <div className="dl-body">
          <aside className="dl-side">
            {VIEWS.map(({ key, label, icon: Icon }) => (
              <Fragment key={key}>
                {key === 'all' && <hr className="nb-side-sep" />}
                <button className={`dl-list nb-side-item${view === key ? ' active' : ''}`} onClick={() => set({ view: key === 'mine' ? null : key, offset: null })}>
                  <Icon size={17} /> {label}
                </button>
              </Fragment>
            ))}
            <hr className="nb-side-sep" />
            <Link className="dl-list nb-side-item" to="/notebooks/templates">
              <LayoutTemplate size={15} /> Templates
            </Link>
          </aside>

          <main className="dl-main">
            <div className="nb-filters">
              <label className="nb-search">
                <Search size={16} />
                <input placeholder="Search by title, keyword or author" value={query} onChange={(e) => set({ q: e.target.value, offset: null })} />
              </label>
              <Select
                label="Author"
                value={author ?? ''}
                options={[{ value: '', label: 'Any author' }, ...(search.data?.authors ?? []).map((a) => ({ value: a.id, label: a.name }))]}
                onChange={(v) => set({ author: v, offset: null })}
              />
              <Select
                label="Team"
                value={team ?? ''}
                options={[{ value: '', label: 'Any team' }, ...(search.data?.teams ?? []).map((t) => ({ value: t, label: t }))]}
                onChange={(v) => set({ team: v, offset: null })}
              />
            </div>
            {view === 'mine' && !query && (
              <section className="nb-get-started">
                <div className="nb-get-started-head">
                  <h2>Browse our templates</h2>
                  <Link to="/notebooks/templates">View All Templates ›</Link>
                </div>
                <div className="nb-templates">
                  {NOTEBOOK_TEMPLATES.slice(0, 3).map((t) => (
                    <TemplateCard key={t.id} template={t} onUse={() => !template.pending && template.use(t)} />
                  ))}
                </div>
              </section>
            )}

            <h2 className="dl-h2">{view === 'mine' ? 'Created by me' : VIEWS.find((v) => v.key === view)?.label}</h2>
            <div className="dl-table-card">
              <div className="dl-table-bar">
                <strong>{total ? `${offset + 1}–${Math.min(offset + PAGE, total)}` : '0'}</strong> of <strong>{total.toLocaleString()}</strong> notebooks
              </div>
              {search.isPending ? (
                <Loading label="Loading notebooks…" />
              ) : search.isError ? (
                <ErrorState error={search.error} onRetry={() => search.refetch()} />
              ) : rows.length === 0 ? (
                <Empty>{view === 'deleted' ? 'No recently deleted notebooks.' : query || author || team ? 'No notebooks match.' : 'No notebooks here yet.'}</Empty>
              ) : (
                <table className={`dl-table${search.isPlaceholderData ? ' is-stale' : ''}`}>
                  <thead>
                    <tr>
                      <th className="dl-starcol">
                        <Star size={16} fill="currentColor" />
                      </th>
                      {th('NAME', 'title')}
                      {th('AUTHOR')}
                      {th('TEAMS')}
                      {th(view === 'recents' ? 'VIEWED' : 'MODIFIED', 'modified')}
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((nb) => (
                      <tr key={nb.id} onClick={() => navigate(`/notebooks/${nb.id}`)}>
                        <td className="dl-starcol">
                          <FavoriteButton nb={nb} />
                        </td>
                        <td className="dl-name">
                          <span className="dl-name-inner">
                            <BookOpen size={16} className="muted" />
                            {nb.title}
                            {nb.status === 'draft' && <span className="dl-badge">Draft</span>}
                          </span>
                          <span className="nb-row-meta">
                            {nb.cellCount} cell{nb.cellCount === 1 ? '' : 's'}
                            {nb.tags.map((t) => (
                              <span key={t} className="tag-chip">
                                {t}
                              </span>
                            ))}
                          </span>
                        </td>
                        <td>
                          {nb.author && (
                            <span className="dl-author">
                              <Initials name={nb.author.name} /> {nb.author.name}
                            </span>
                          )}
                        </td>
                        <td>
                          {nb.teams.map((t) => (
                            <span key={t} className="tag-chip dl-team">
                              {t}
                            </span>
                          ))}
                        </td>
                        <td>
                          <div>{timeAgo(view === 'recents' && nb.viewedAt ? nb.viewedAt : view === 'deleted' && nb.deletedAt ? nb.deletedAt : nb.modifiedAt)}</div>
                          <div className="muted-text dl-created">Created {new Date(nb.createdAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}</div>
                        </td>
                        <td onClick={(e) => e.stopPropagation()}>
                          <Menu
                            triggerClassName="icon-plain"
                            trigger={<EllipsisVertical size={16} />}
                            items={
                              view === 'deleted'
                                ? [{ label: 'Restore', onClick: () => restore.mutate(nb.id) }]
                                : [
                                    { label: 'Open', onClick: () => navigate(`/notebooks/${nb.id}`) },
                                    { label: 'Clone', onClick: () => clone.mutate(nb.id, { onSuccess: (c) => navigate(`/notebooks/${c.id}`) }) },
                                    { label: 'Delete', onClick: () => del.mutate(nb.id) },
                                  ]
                            }
                          />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
              <Pager offset={offset} limit={PAGE} total={total} onChange={(o) => set({ offset: o ? String(o) : null })} />
            </div>
          </main>
        </div>
      </div>
    </div>
  );
}

/** Templates tab: every built-in template with its outline. */
export function NotebookTemplatesPage() {
  const template = useUseTemplate();
  return (
    <div className="dl-page">
      <NotebooksHeader />
      <div className="dl-scroll">
        <h2 className="dl-h2">Templates</h2>
        <p className="muted-text nb-intro">Start a notebook from a structure your team already knows. The new notebook is yours to edit; the template stays as it is.</p>
        <div className="nb-templates nb-templates-wide">
          {NOTEBOOK_TEMPLATES.map((t) => (
            <TemplateCard key={t.id} template={t} detailed onUse={() => !template.pending && template.use(t)} />
          ))}
        </div>
      </div>
    </div>
  );
}

/** Reports tab: scheduled notebook reports are not available yet. */
export function NotebookReportsPage() {
  return (
    <div className="dl-page">
      <NotebooksHeader />
      <Empty>No reports yet. Scheduled reports that email a notebook to your team will be listed here.</Empty>
    </div>
  );
}
