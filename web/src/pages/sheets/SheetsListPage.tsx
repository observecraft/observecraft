import { useCallback, useDeferredValue, useEffect, useRef, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ChevronDown, ChevronRight, EllipsisVertical, FileSpreadsheet, PanelLeft, PanelLeftClose, Pin, Plus, Search, Table2, User } from 'lucide-react';
import type { SpreadsheetSummary, SpreadsheetView } from '../../api/sheets/types';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { Initials } from '../dashboards/list/DashboardListPage';
import { useCreateSpreadsheet, useDeleteSpreadsheets, useSpreadsheetSearch, useUpdateSpreadsheet } from './queries';
import '../dashboards/dashboards.css';
import './sheets.css';

const VIEWS: { key: SpreadsheetView; label: string; icon: typeof User }[] = [
  { key: 'all', label: 'All Spreadsheets', icon: Table2 },
  { key: 'mine', label: 'My Spreadsheets', icon: User },
];

/** Creates an untitled spreadsheet and opens it. */
function useNewSpreadsheet() {
  const create = useCreateSpreadsheet();
  const navigate = useNavigate();
  return { pending: create.isPending, start: () => create.mutate({ name: 'Untitled spreadsheet' }, { onSuccess: (s) => navigate(`/sheets/${s.id}`) }) };
}

export function SheetsListPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const view = (params.get('view') as SpreadsheetView | null) ?? 'all';
  const team = params.get('team');
  const set = useCallback(
    (u: Record<string, string | null>) =>
      setParams(
        (p) => {
          const n = new URLSearchParams(p);
          for (const [k, v] of Object.entries(u)) {
            if (v) n.set(k, v);
            else n.delete(k);
          }
          return n;
        },
        { replace: true },
      ),
    [setParams],
  );
  const [showControls, setShowControls] = useState(true);
  const [teamsOpen, setTeamsOpen] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [renaming, setRenaming] = useState<SpreadsheetSummary | null>(null);
  const [name, setName] = useState('');
  const search = useSpreadsheetSearch({ query: useDeferredValue(query), view, team });
  const del = useDeleteSpreadsheets();
  const update = useUpdateSpreadsheet();
  const fresh = useNewSpreadsheet();

  // ?new=1 (the menus' "+New Spreadsheet") creates one once and opens it.
  const started = useRef(false);
  const { start } = fresh;
  useEffect(() => {
    if (params.get('new') !== '1' || started.current) return;
    started.current = true;
    start();
  }, [params, start]);

  const rows = search.data?.spreadsheets ?? [];
  const allSelected = rows.length > 0 && rows.every((r) => selected.has(r.id));
  const toggle = (id: string) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  return (
    <div className="dl-page">
      <header className="product-header">
        <div className="product-title">
          <FileSpreadsheet size={20} /> Sheets
        </div>
        <div className="dl-header-actions">
          <button className="btn btn-primary" disabled={fresh.pending} onClick={fresh.start}>
            <Plus size={14} /> New Spreadsheet
          </button>
        </div>
      </header>
      <div className="dl-scroll">
        <label className="int-search dl-search">
          <Search size={16} />
          <input placeholder="Search All Spreadsheets" value={query} onChange={(e) => set({ q: e.target.value })} />
        </label>
        <div className="dl-body">
          {showControls && (
            <aside className="dl-side">
              <button className="link-btn dl-hide" onClick={() => setShowControls(false)}>
                <PanelLeftClose size={15} /> Hide Controls
              </button>
              <div className="et-side-card" onClick={() => setTeamsOpen((o) => !o)}>
                <span className="et-side-toggle">{teamsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />} Teams</span>
                <Pin size={14} className="muted" />
              </div>
              {teamsOpen && (
                <ul className="et-team-list">
                  {(search.data?.teams ?? []).map((t) => (
                    <li key={t}>
                      <label>
                        <input type="radio" name="ss-team" checked={team === t} onChange={() => set({ team: t })} /> {t}
                      </label>
                    </li>
                  ))}
                  {search.data && search.data.teams.length === 0 && <li className="muted-text">No team has spreadsheets yet.</li>}
                  {team && (
                    <li>
                      <button className="link-btn" onClick={() => set({ team: null })}>
                        All teams
                      </button>
                    </li>
                  )}
                </ul>
              )}
              {VIEWS.map((v) => (
                <button key={v.key} className={`dl-list${view === v.key ? ' active' : ''}`} onClick={() => set({ view: v.key === 'all' ? null : v.key })}>
                  <v.icon size={15} /> <span className="dl-list-name">{v.label}</span>
                </button>
              ))}
            </aside>
          )}
          <main className="dl-main">
            {!showControls && (
              <button className="link-btn dl-hide" onClick={() => setShowControls(true)}>
                <PanelLeft size={15} /> Show Controls
              </button>
            )}
            <h2 className="dl-h2">{VIEWS.find((v) => v.key === view)?.label}</h2>
            <div className="dl-table-card">
              <div className="dl-table-bar">
                <strong>{search.data?.total ?? 0}</strong> spreadsheets
                <span className="dl-sep" />
                {selected.size === 0 ? (
                  <span className="muted-text">Select rows to take action</span>
                ) : (
                  <span className="dl-bulk">
                    {selected.size} selected
                    <button className="btn btn-sm btn-danger" onClick={() => del.mutate([...selected], { onSuccess: () => setSelected(new Set()) })}>
                      Delete
                    </button>
                  </span>
                )}
              </div>
              {search.isPending ? (
                <Loading label="Loading spreadsheets…" />
              ) : search.isError ? (
                <ErrorState error={search.error} onRetry={() => search.refetch()} />
              ) : rows.length === 0 ? (
                <Empty>No Spreadsheets Found</Empty>
              ) : (
                <table className={`dl-table${search.isPlaceholderData ? ' is-stale' : ''}`}>
                  <thead>
                    <tr>
                      <th className="dl-check">
                        <input
                          type="checkbox"
                          className="checkbox"
                          aria-label="Select all"
                          checked={allSelected}
                          onChange={() => setSelected(allSelected ? new Set() : new Set(rows.map((r) => r.id)))}
                        />
                      </th>
                      <th>NAME</th>
                      <th>AUTHOR</th>
                      <th>TEAM</th>
                      <th>MODIFIED</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((s) => (
                      <tr key={s.id} onClick={() => navigate(`/sheets/${s.id}`)}>
                        <td className="dl-check" onClick={(e) => e.stopPropagation()}>
                          <input type="checkbox" className="checkbox" aria-label={`Select ${s.name}`} checked={selected.has(s.id)} onChange={() => toggle(s.id)} />
                        </td>
                        <td className="dl-name">
                          <span className="dl-name-inner">
                            <FileSpreadsheet size={16} className="muted" /> {s.name}
                            <span className="muted-text ss-count">
                              {s.sheetCount} sheet{s.sheetCount === 1 ? '' : 's'}
                            </span>
                          </span>
                        </td>
                        <td>
                          {s.author && (
                            <span className="dl-author">
                              <Initials name={s.author.name} /> {s.author.name.toUpperCase()}
                            </span>
                          )}
                        </td>
                        <td>{s.team && <span className="tag-chip dl-team">{s.team}</span>}</td>
                        <td>
                          <div>{timeAgo(s.modifiedAt)}</div>
                          <div className="muted-text dl-created">Created {new Date(s.createdAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}</div>
                        </td>
                        <td onClick={(e) => e.stopPropagation()}>
                          <Menu
                            triggerClassName="icon-plain"
                            trigger={<EllipsisVertical size={16} />}
                            items={[
                              { label: 'Open', onClick: () => navigate(`/sheets/${s.id}`) },
                              { label: 'Rename', onClick: () => (setRenaming(s), setName(s.name)) },
                              { label: 'Delete', onClick: () => del.mutate([s.id]) },
                            ]}
                          />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </main>
        </div>
      </div>
      {renaming && (
        <Modal onClose={() => setRenaming(null)} label="Rename spreadsheet" width={420} height="auto">
          <div className="small-modal">
            <h3>Rename spreadsheet</h3>
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && name.trim() && update.mutate({ id: renaming.id, patch: { name } }, { onSuccess: () => setRenaming(null) })}
            />
            {update.error && <p className="form-error">{update.error.message}</p>}
            <div className="small-modal-actions">
              <button className="btn" onClick={() => setRenaming(null)}>
                Cancel
              </button>
              <button
                className="btn btn-primary"
                disabled={!name.trim() || update.isPending}
                onClick={() => update.mutate({ id: renaming.id, patch: { name } }, { onSuccess: () => setRenaming(null) })}
              >
                Rename
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}
