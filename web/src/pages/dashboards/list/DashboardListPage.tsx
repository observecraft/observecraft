import { useCallback, useDeferredValue, useEffect, useState } from 'react';
import { Link, NavLink, useNavigate, useSearchParams } from 'react-router-dom';
import { ChevronDown, ChevronRight, EllipsisVertical, LayoutDashboard, PanelLeft, PanelLeftClose, Pin, Plus, Search, Siren, Star } from 'lucide-react';
import type { DashboardSummary, PresetList } from '../../../api/dashboards/types';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Menu, Select } from '../../../components/ui/Dropdown';
import { Modal } from '../../../components/ui/Modal';
import { Pager } from '../../../components/ui/Pager';
import { timeAgo } from '../../../lib/format';
import { IntegrationLogo } from '../../integrations/IntegrationLogo';
import { useAddToList, useCloneDashboard, useCreateList, useDashboardSearch, useDeleteDashboards, useLists, useRecentlyViewed, useRestoreDashboard, useUpdateDashboardStar } from '../queries';
import { CreateDashboardModal } from './CreateDashboardModal';
import '../dashboards.css';

const PAGE = 50;

const PRESETS: { key: PresetList; label: string }[] = [
  { key: 'custom', label: 'All Custom' },
  { key: 'integrations', label: 'All Integrations' },
  { key: 'mine', label: 'Created By You' },
  { key: 'starred', label: 'Starred' },
  { key: 'deleted', label: 'Recently Deleted' },
];

function DashIcon({ d, size = 18 }: { d: Pick<DashboardSummary, 'logo' | 'title'>; size?: number }) {
  if (d.logo === 'tile:monitors') return <Siren size={size} color="var(--status-warn)" />;
  if (d.logo) return <IntegrationLogo logo={d.logo} name={d.title} size={size} />;
  return null;
}

export function Initials({ name }: { name: string }) {
  const colors = ['#0b6e69', '#2f6ad0', '#29a8a0', '#d65cad', '#f2a93b'];
  const c = colors[[...name].reduce((a, ch) => a + ch.charCodeAt(0), 0) % colors.length];
  return (
    <span className="dl-avatar" style={{ background: c }}>
      {name
        .split(' ')
        .map((p) => p[0])
        .join('')
        .slice(0, 2)}
    </span>
  );
}

function Popularity({ value }: { value: number }) {
  return (
    <span className="dl-pop" title={`Popularity ${value}/5`}>
      {[0, 1, 2, 3, 4].map((i) => (
        <span key={i} className={i < value ? 'on' : ''} />
      ))}
    </span>
  );
}

function StarButton({ d }: { d: DashboardSummary }) {
  const star = useUpdateDashboardStar();
  return (
    <button className={`icon-plain dl-star${d.starred ? ' on' : ''}`} title={d.starred ? 'Unstar' : 'Star'} onClick={(e) => (e.preventDefault(), star.mutate({ id: d.id, starred: !d.starred }))}>
      <Star size={17} fill={d.starred ? 'currentColor' : 'none'} />
    </button>
  );
}

export function DashboardListPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const preset = (params.get('preset') as PresetList | null) ?? 'all';
  const listId = params.get('list');
  const team = params.get('team');
  const [sortField, sortDir] = (params.get('sort') ?? 'modified:desc').split(':') as ['title' | 'modified' | 'popularity', 'asc' | 'desc'];
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

  const [showControls, setShowControls] = useState(true);
  const [teamsOpen, setTeamsOpen] = useState(false);
  const [listFilter, setListFilter] = useState('');
  const [listsExpanded, setListsExpanded] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [creating, setCreating] = useState(false);
  const [newList, setNewList] = useState(false);
  const [listName, setListName] = useState('');
  const [addingTo, setAddingTo] = useState<string[] | null>(null);
  // ?new=1 (the sidebar's "New Dashboard") opens the create dialog once.
  const wantsNew = params.get('new') === '1';
  useEffect(() => {
    if (!wantsNew) return;
    setCreating(true);
    set({ new: null });
  }, [wantsNew, set]);

  const search = useDashboardSearch({ query: useDeferredValue(query), preset, listId, team, sort: { field: sortField, dir: sortDir }, offset, limit: PAGE });
  const recent = useRecentlyViewed();
  const lists = useLists();
  const del = useDeleteDashboards();
  const restore = useRestoreDashboard();
  const clone = useCloneDashboard();
  const createList = useCreateList();
  const addToList = useAddToList();

  const rows = search.data?.dashboards ?? [];
  const total = search.data?.total ?? 0;
  const allSelected = rows.length > 0 && rows.every((r) => selected.has(r.id));
  const shownLists = (lists.data ?? []).filter((l) => l.name.toLowerCase().includes(listFilter.toLowerCase()));

  const sortBy = (field: typeof sortField) => set({ sort: `${field}:${field === sortField && sortDir === 'desc' ? 'asc' : 'desc'}`, offset: null });
  const th = (label: string, field?: typeof sortField) => (
    <th className={field ? 'sortable' : undefined} onClick={field ? () => sortBy(field) : undefined}>
      {label}
      {field === sortField && (sortDir === 'desc' ? ' ↓' : ' ↑')}
    </th>
  );

  return (
    <div className="dl-page">
      <header className="product-header">
        <div className="product-title">
          <LayoutDashboard size={20} /> Dashboards
        </div>
        <nav className="product-tabs">
          <NavLink to="/dashboards" end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            List
          </NavLink>
          <NavLink to="/dashboards/shared" className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Shared Dashboards
          </NavLink>
        </nav>
        <div className="dl-header-actions">
          <button className="btn" onClick={() => setNewList(true)}>
            <Plus size={14} /> New List
          </button>
          <button className="btn btn-primary" onClick={() => setCreating(true)}>
            <Plus size={14} /> New Dashboard
          </button>
        </div>
      </header>

      <div className="dl-scroll">
        <label className="int-search dl-search">
          <Search size={16} />
          <input placeholder="Search dashboards" value={query} onChange={(e) => set({ q: e.target.value, offset: null })} />
        </label>

        <div className="dl-body">
          {showControls && (
            <aside className="dl-side">
              <button className="link-btn dl-hide" onClick={() => setShowControls(false)}>
                <PanelLeftClose size={15} /> Hide Controls
              </button>
              <div className="et-side-card" onClick={() => setTeamsOpen((o) => !o)}>
                <span className="et-side-toggle">
                  {teamsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />} Teams
                </span>
                <Pin size={14} className="muted" />
              </div>
              {teamsOpen && (
                <ul className="et-team-list">
                  {['storefront', 'billing', 'platform', 'checkout'].map((t) => (
                    <li key={t}>
                      <label>
                        <input type="radio" name="dl-team" checked={team === t} onChange={() => set({ team: t, offset: null })} /> {t}
                      </label>
                    </li>
                  ))}
                  <li>
                    <button className="link-btn" onClick={() => set({ team: null })}>
                      All teams
                    </button>
                  </li>
                </ul>
              )}
              <label className="facet-search dl-filter">
                <Search size={14} />
                <input placeholder="Filter" value={listFilter} onChange={(e) => setListFilter(e.target.value)} />
              </label>
              <h4 className="dl-side-h">
                <ChevronDown size={14} /> Preset Lists
              </h4>
              <button className={`dl-list${preset === 'all' && !listId ? ' active' : ''}`} onClick={() => set({ preset: null, list: null, offset: null })}>
                <Star size={15} /> All Dashboards
              </button>
              {PRESETS.filter((p) => p.label.toLowerCase().includes(listFilter.toLowerCase())).map((p) => (
                <button key={p.key} className={`dl-list${preset === p.key ? ' active' : ''}`} onClick={() => set({ preset: p.key, list: null, offset: null })}>
                  <Star size={15} /> {p.label}
                </button>
              ))}
              <h4 className="dl-side-h">
                <ChevronDown size={14} /> Shared, Editable Lists
              </h4>
              {(listsExpanded ? shownLists : shownLists.slice(0, 6)).map((l) => (
                <button key={l.id} className={`dl-list${listId === l.id ? ' active' : ''}`} onClick={() => set({ list: l.id, preset: null, offset: null })}>
                  <Star size={15} /> <span className="dl-list-name">{l.name}</span>
                  <span className="dl-list-count">{l.dashboardIds.length}</span>
                </button>
              ))}
              {shownLists.length > 6 && (
                <button className="dl-more" onClick={() => setListsExpanded((x) => !x)}>
                  {listsExpanded ? 'View less' : 'View more'} <ChevronDown size={14} className={listsExpanded ? 'rot180' : ''} />
                </button>
              )}
            </aside>
          )}

          <main className="dl-main">
            {!showControls && (
              <button className="link-btn dl-hide" onClick={() => setShowControls(true)}>
                <PanelLeft size={15} /> Show Controls
              </button>
            )}
            {!query && preset === 'all' && !listId && (recent.data?.length ?? 0) > 0 && (
              <section>
                <h2 className="dl-h2">Recently Viewed</h2>
                <div className="dl-recent">
                  {recent.data!.map((d) => (
                    <Link key={d.id} to={`/dashboards/${d.id}`} className="dl-card">
                      <div className="dl-card-top">
                        <DashIcon d={d} size={26} />
                        <StarButton d={d} />
                        <div>
                          <div className="dl-card-title">{d.title}</div>
                          <div className="muted-text">Viewed {timeAgo(d.viewedAt!)}</div>
                        </div>
                      </div>
                      <div className="dl-card-foot">
                        {d.kind === 'integration' ? (
                          'Integration Dashboard'
                        ) : (
                          <>
                            {d.author && <Initials name={d.author.name} />} Created by {d.author?.name.toUpperCase() ?? 'unknown'}
                            {d.teams[0] && <span className="tag-chip dl-team">{d.teams[0]}</span>}
                          </>
                        )}
                      </div>
                    </Link>
                  ))}
                </div>
              </section>
            )}

            <h2 className="dl-h2">{listId ? lists.data?.find((l) => l.id === listId)?.name : PRESETS.find((p) => p.key === preset)?.label ?? 'All Dashboards'}</h2>
            <div className="dl-table-card">
              <div className="dl-table-bar">
                <strong>{total ? `${offset + 1}–${Math.min(offset + PAGE, total)}` : '0'}</strong> of <strong>{total.toLocaleString()}</strong> dashboards
                <span className="dl-sep" />
                {selected.size === 0 ? (
                  <span className="muted-text">Select rows to take action</span>
                ) : (
                  <span className="dl-bulk">
                    {selected.size} selected
                    <button className="btn btn-sm" onClick={() => setAddingTo([...selected])}>
                      Add to list
                    </button>
                    {preset !== 'deleted' && (
                      <button className="btn btn-sm btn-danger" onClick={() => del.mutate([...selected], { onSuccess: () => setSelected(new Set()) })}>
                        Delete
                      </button>
                    )}
                  </span>
                )}
              </div>
              {search.isPending ? (
                <Loading label="Loading dashboards…" />
              ) : search.isError ? (
                <ErrorState error={search.error} onRetry={() => search.refetch()} />
              ) : rows.length === 0 ? (
                <Empty>{preset === 'deleted' ? 'No recently deleted dashboards.' : 'No dashboards match.'}</Empty>
              ) : (
                <table className={`dl-table${search.isPlaceholderData ? ' is-stale' : ''}`}>
                  <thead>
                    <tr>
                      <th className="dl-check">
                        <input type="checkbox" className="checkbox" checked={allSelected} onChange={() => setSelected(allSelected ? new Set() : new Set(rows.map((r) => r.id)))} />
                      </th>
                      <th className="dl-starcol">
                        <Star size={16} fill="currentColor" />
                      </th>
                      {th('NAME', 'title')}
                      {th('AUTHOR')}
                      {th('TEAMS')}
                      {th('POPULARITY', 'popularity')}
                      {th('MODIFIED', 'modified')}
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((d) => (
                      <tr key={d.id} onClick={() => preset !== 'deleted' && navigate(`/dashboards/${d.id}`)}>
                        <td className="dl-check" onClick={(e) => e.stopPropagation()}>
                          <input
                            type="checkbox"
                            className="checkbox"
                            checked={selected.has(d.id)}
                            onChange={() =>
                              setSelected((s) => {
                                const n = new Set(s);
                                if (n.has(d.id)) n.delete(d.id);
                                else n.add(d.id);
                                return n;
                              })
                            }
                          />
                        </td>
                        <td className="dl-starcol" onClick={(e) => e.stopPropagation()}>
                          <StarButton d={d} />
                        </td>
                        <td className="dl-name">
                          <span className="dl-name-inner">
                            <DashIcon d={d} />
                            {d.title}
                            {d.kind === 'integration' && <span className="dl-badge">Integration</span>}
                          </span>
                        </td>
                        <td>
                          {d.author && (
                            <span className="dl-author">
                              <Initials name={d.author.name} /> {d.author.name.toUpperCase()}
                            </span>
                          )}
                        </td>
                        <td>
                          {d.teams.map((t) => (
                            <span key={t} className="tag-chip dl-team">
                              {t}
                            </span>
                          ))}
                        </td>
                        <td>
                          <Popularity value={d.popularity} />
                        </td>
                        <td>
                          {d.kind === 'custom' && (
                            <>
                              <div>{timeAgo(d.modifiedAt)}</div>
                              <div className="muted-text dl-created">Created {new Date(d.createdAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}</div>
                            </>
                          )}
                        </td>
                        <td onClick={(e) => e.stopPropagation()}>
                          <Menu
                            triggerClassName="icon-plain"
                            trigger={<EllipsisVertical size={16} />}
                            items={
                              preset === 'deleted'
                                ? [{ label: 'Restore', onClick: () => restore.mutate(d.id) }]
                                : [
                                    { label: 'Open', onClick: () => navigate(`/dashboards/${d.id}`) },
                                    { label: 'Clone', onClick: () => clone.mutate(d.id, { onSuccess: (c) => navigate(`/dashboards/${c.id}`) }) },
                                    { label: 'Add to list…', onClick: () => setAddingTo([d.id]) },
                                    { label: 'Delete', onClick: () => del.mutate([d.id]), disabled: d.kind === 'integration' },
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

      {creating && <CreateDashboardModal onClose={() => setCreating(false)} />}

      {newList && (
        <Modal onClose={() => setNewList(false)} label="New list" width={420} height="auto">
          <div className="small-modal">
            <h3>New dashboard list</h3>
            <input autoFocus placeholder="List name" value={listName} onChange={(e) => setListName(e.target.value)} />
            <div className="small-modal-actions">
              <button className="btn" onClick={() => setNewList(false)}>
                Cancel
              </button>
              <button
                className="btn btn-primary"
                disabled={!listName.trim()}
                onClick={() =>
                  createList.mutate(listName.trim(), {
                    onSuccess: (l) => {
                      setNewList(false);
                      setListName('');
                      set({ list: l.id, preset: null });
                    },
                  })
                }
              >
                Create
              </button>
            </div>
          </div>
        </Modal>
      )}

      {addingTo && (
        <Modal onClose={() => setAddingTo(null)} label="Add to list" width={420} height="auto">
          <div className="small-modal">
            <h3>
              Add {addingTo.length} dashboard{addingTo.length > 1 ? 's' : ''} to a list
            </h3>
            <Select
              value=""
              options={[{ value: '', label: 'Choose a list' }, ...(lists.data ?? []).map((l) => ({ value: l.id, label: l.name }))]}
              onChange={(lid) =>
                lid &&
                addToList.mutate(
                  { listId: lid, ids: addingTo },
                  {
                    onSuccess: () => {
                      setAddingTo(null);
                      setSelected(new Set());
                    },
                  },
                )
              }
            />
          </div>
        </Modal>
      )}
    </div>
  );
}

export function SharedDashboardsPage() {
  return (
    <div className="dl-page">
      <header className="product-header">
        <div className="product-title">
          <LayoutDashboard size={20} /> Dashboards
        </div>
        <nav className="product-tabs">
          <NavLink to="/dashboards" end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            List
          </NavLink>
          <NavLink to="/dashboards/shared" className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Shared Dashboards
          </NavLink>
        </nav>
      </header>
      <Empty>No dashboards are shared publicly. Public sharing links will be listed here once sharing is enabled.</Empty>
    </div>
  );
}
