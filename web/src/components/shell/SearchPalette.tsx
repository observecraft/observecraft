import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { ArrowDown, ArrowUp, Bell, CornerDownLeft, LayoutDashboard, Plus, Search, Settings, User, Users, X, type LucideIcon } from 'lucide-react';
import { searchDashboards } from '../../api/dashboards/client';
import { searchMonitors } from '../../api/monitors/client';
import { searchTeams, searchUsers } from '../../api/identity/client';
import { NAV_FOOTER, NAV_SECTIONS } from './navMenus';
import { buildPageIndex, entriesForChip, rankEntries, type PageChip, type SearchEntry } from './searchIndex';

type EntityChip = 'dashboard' | 'monitor' | 'team' | 'user';
type Chip = PageChip | EntityChip;

const CHIPS: { key: Chip; label: string; icon: LucideIcon }[] = [
  { key: 'actions', label: 'Actions', icon: Plus },
  { key: 'settings', label: 'Settings', icon: Settings },
  { key: 'dashboard', label: 'Dashboard', icon: LayoutDashboard },
  { key: 'monitor', label: 'Monitor', icon: Bell },
  { key: 'team', label: 'Team', icon: Users },
  { key: 'user', label: 'User', icon: User },
];

const PAGES = buildPageIndex(NAV_SECTIONS, NAV_FOOTER);
// Each page shows the icon of the sidebar item it sits under.
const PAGE_ICONS = new Map<string, LucideIcon>(
  [...NAV_SECTIONS.flat(), ...NAV_FOOTER].flatMap((i) => [
    [i.to, i.icon] as const,
    ...(i.menu ?? []).flatMap((g) => [...(g.to ? [g.to] : []), ...g.columns.flat().map((l) => l.to)].map((to) => [to, i.icon] as const)),
  ]),
);
const KIND_ICONS: Partial<Record<SearchEntry['kind'], LucideIcon>> = { action: Plus, dashboard: LayoutDashboard, monitor: Bell, team: Users, user: User };

const isPageChip = (c: Chip | null): c is PageChip | null => c === null || c === 'actions' || c === 'settings';

/** Dashboards, monitors, teams or users matching the query, as search entries. */
async function searchEntities(kind: EntityChip, query: string, limit: number): Promise<SearchEntry[]> {
  if (kind === 'dashboard') {
    const page = await searchDashboards({ query, preset: 'all', listId: null, team: null, sort: { field: 'popularity', dir: 'desc' }, offset: 0, limit });
    return page.dashboards.map((d) => ({ kind, name: d.title, category: 'Dashboard', path: `/dashboards/${d.id}` }));
  }
  if (kind === 'monitor') {
    const page = await searchMonitors({ query, offset: 0, limit, sort: 'name', creators: [], drafts: [false], muted: [], priorities: [], recipients: [], statuses: [], teams: [], types: [] });
    return page.monitors.map((m) => ({ kind, name: m.name, category: 'Monitor', path: `/monitors/${m.id}` }));
  }
  if (kind === 'team') {
    const page = await searchTeams({ query, limit, offset: 0, sort: 'name' });
    return page.teams.map((t) => ({ kind, name: t.name, category: `Team · @${t.handle}`, path: '/teams' }));
  }
  const page = await searchUsers({ query, limit, offset: 0 });
  return page.users.map((u) => ({ kind, name: u.name || u.email, category: `User · ${u.email}`, path: '/organization-settings/users' }));
}

function Key({ children }: { children: ReactNode }) {
  return <kbd className="sp-key">{children}</kbd>;
}

/** The top-bar search: pages of every product, create actions and entities, with filter chips and keyboard navigation. */
export function SearchPalette({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const [query, setQuery] = useState('');
  const [chip, setChip] = useState<Chip | null>(null);
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const q = query.trim();

  // Entities: the chip's kind, or (no chip) dashboards and monitors once something is typed.
  const entityKinds: EntityChip[] = isPageChip(chip) ? (chip === null && q.length >= 2 ? ['dashboard', 'monitor'] : []) : [chip];
  const entities = useQuery({
    queryKey: ['search-palette', entityKinds, q],
    queryFn: async () => (await Promise.all(entityKinds.map((k) => searchEntities(k, q, chip ? 50 : 5)))).flat(),
    enabled: entityKinds.length > 0,
    staleTime: 30_000,
    placeholderData: (prev) => prev,
  });
  const hasEntities = entityKinds.length > 0;

  const results = useMemo(() => {
    const pages = isPageChip(chip) ? entriesForChip(PAGES, chip) : [];
    return rankEntries([...pages, ...(hasEntities ? (entities.data ?? []) : [])], q, 50);
  }, [chip, q, hasEntities, entities.data]);

  useEffect(() => setActive(0), [chip, q]);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  const open = (e: SearchEntry) => {
    onClose();
    navigate(e.path);
  };

  const onKeyDown = (ev: React.KeyboardEvent) => {
    if (ev.key === 'ArrowDown') {
      ev.preventDefault();
      setActive((i) => Math.min(i + 1, results.length - 1));
    } else if (ev.key === 'ArrowUp') {
      ev.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (ev.key === 'Enter' && results[active]) {
      ev.preventDefault();
      open(results[active]);
    } else if (ev.key === 'Escape') {
      onClose();
    } else if (ev.key === 'Tab') {
      // Tab steps through the chips (Shift+Tab backwards); past the last one, no chip.
      ev.preventDefault();
      const keys: (Chip | null)[] = [null, ...CHIPS.map((c) => c.key)];
      const at = keys.indexOf(chip);
      setChip(keys[(at + (ev.shiftKey ? keys.length - 1 : 1)) % keys.length]);
    }
  };

  return (
    <div className="sp-backdrop" onMouseDown={onClose}>
      <div className="sp-panel" role="dialog" aria-label="Search" onMouseDown={(e) => e.stopPropagation()} onKeyDown={onKeyDown}>
        <div className="sp-input">
          <Search size={16} />
          <input autoFocus value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search for products and entities…" aria-label="Search for products and entities" />
          <button type="button" className="sp-close" onClick={onClose} aria-label="Close search">
            <X size={16} />
          </button>
        </div>
        <div className="sp-chips">
          {CHIPS.map(({ key, label, icon: Icon }) => (
            <button key={key} type="button" className={`sp-chip${chip === key ? ' active' : ''}`} onClick={() => setChip(chip === key ? null : key)}>
              <Icon size={13} />
              {label}
            </button>
          ))}
        </div>
        <div className="sp-results" ref={listRef} role="listbox">
          {results.map((e, i) => {
            const Icon = KIND_ICONS[e.kind] ?? PAGE_ICONS.get(e.path) ?? Search;
            return (
              <button
                key={`${e.kind}:${e.path}:${e.name}`}
                type="button"
                role="option"
                aria-selected={i === active}
                data-index={i}
                className={`sp-row${i === active ? ' active' : ''}`}
                onMouseMove={() => i !== active && setActive(i)}
                onClick={() => open(e)}
              >
                <Icon size={15} className="sp-row-icon" />
                <span className="sp-row-name">{e.name}</span>
                <span className="sp-row-category">{e.category}</span>
                <span className="sp-row-path">{e.path}</span>
              </button>
            );
          })}
          {!results.length && <div className="sp-empty">{entities.isFetching ? 'Searching…' : `No results for “${q}”`}</div>}
        </div>
        <div className="sp-footer">
          <span>Navigate</span>
          <Key>
            <ArrowDown size={11} />
          </Key>
          <Key>
            <ArrowUp size={11} />
          </Key>
          <span className="sp-footer-sep" />
          <span>Open</span>
          <Key>
            <CornerDownLeft size={11} />
          </Key>
          <span className="sp-footer-sep" />
          <span>Filter</span>
          <Key>tab</Key>
          <span className="sp-footer-sep" />
          <span>Close</span>
          <Key>esc</Key>
        </div>
      </div>
    </div>
  );
}
