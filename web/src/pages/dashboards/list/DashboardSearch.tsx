import { useRef, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router-dom';
import { Clock, LayoutDashboard, Search } from 'lucide-react';
import type { DashboardSummary } from '../../../api/dashboards/types';
import { SEARCH_FILTERS, addRecentSearch, type RecentSearch } from '../../../api/dashboards/search';
import { UrlSearchInput } from '../../../components/ui/UrlSearchInput';
import { timeAgo } from '../../../lib/format';
import '../../../components/query/queryBar.css';

const RECENT_KEY = 'oc.dashboards.recentSearches';

function loadRecent(): RecentSearch[] {
  try {
    const list = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]');
    return Array.isArray(list) ? list.filter((r) => typeof r?.query === 'string' && typeof r?.at === 'number') : [];
  } catch {
    return [];
  }
}

function saveRecent(list: RecentSearch[]) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(list));
  } catch {
    // Private windows and blocked storage: recent searches just aren't kept.
  }
}

interface Item {
  key: string;
  icon: ReactNode;
  text: ReactNode;
  right?: string;
  run: () => void;
}

/**
 * The dashboard list's search box. Focused, it opens a dropdown: recommended dashboards (recently viewed ones while
 * empty, else the first matches), the search filters (narrowed to the word being typed) and this browser's recent
 * searches; arrows and Enter pick from it.
 */
export function DashboardSearch({
  query,
  onChange,
  matches,
  recentlyViewed,
  icon,
}: {
  query: string;
  onChange: (q: string) => void;
  matches: DashboardSummary[];
  recentlyViewed: DashboardSummary[];
  icon: (d: DashboardSummary) => ReactNode;
}) {
  const navigate = useNavigate();
  const input = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const [recent, setRecent] = useState(loadRecent);
  const remember = (q: string) => {
    const next = addRecentSearch(recent, q, Date.now());
    setRecent(next);
    saveRecent(next);
  };

  // The word being typed: a filter it starts is suggested, a finished filter or plain word isn't.
  const last = query.endsWith(' ') ? '' : (query.split(/\s+/).pop() ?? '');
  const filters = SEARCH_FILTERS.filter((f) => !last || (f.startsWith(last.toLowerCase()) && !last.includes(':')));
  const dashboards = (query.trim() ? matches : recentlyViewed).slice(0, 5);
  const sections: { title: string; items: Item[] }[] = [
    {
      title: 'Recommended Dashboards',
      items: dashboards.map((d) => ({
        key: `d:${d.id}`,
        icon: (
          <>
            <LayoutDashboard size={14} className="muted" />
            {icon(d)}
          </>
        ),
        text: d.title,
        run: () => (query.trim() && remember(query), navigate(`/dashboards/${d.id}`)),
      })),
    },
    {
      title: 'Filter by',
      items: filters.map((f) => ({
        key: `f:${f}`,
        icon: <Search size={14} className="muted" />,
        text: <span className="mono">{f}:</span>,
        run: () => {
          onChange(`${last ? query.slice(0, query.length - last.length) : query && !query.endsWith(' ') ? `${query} ` : query}${f}:`);
          input.current?.focus();
        },
      })),
    },
    {
      title: 'Recent Searches',
      items: recent
        .filter((r) => r.query !== query.trim())
        .map((r) => ({
          key: `r:${r.query}`,
          icon: <Clock size={14} className="muted" />,
          text: <span className="mono">{r.query}</span>,
          right: timeAgo(r.at),
          run: () => (onChange(r.query), remember(r.query), setOpen(false)),
        })),
    },
  ].filter((s) => s.items.length > 0);
  const items = sections.flatMap((s) => s.items);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') setOpen(false);
    else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      setOpen(true);
      const step = e.key === 'ArrowDown' ? 1 : -1;
      // -1 is the input itself: past either end comes back to it.
      setActive((a) => (items.length ? ((a + step + 1 + items.length + 1) % (items.length + 1)) - 1 : -1));
    } else if (e.key === 'Enter') {
      if (open && items[active]) items[active].run();
      else {
        // The input's own text: the URL's `q` may not have caught up with the last keystrokes yet.
        remember(e.currentTarget instanceof HTMLInputElement ? e.currentTarget.value : query);
        setOpen(false);
      }
    }
  };

  return (
    <div className="dl-search-wrap">
      <label className="int-search dl-search">
        <Search size={16} />
        <UrlSearchInput
          ref={input}
          placeholder="Search dashboards"
          value={query}
          onChange={(q) => (onChange(q), setOpen(true), setActive(-1))}
          onFocus={() => (setOpen(true), setActive(-1))}
          onBlur={() => setOpen(false)}
          onKeyDown={onKeyDown}
          aria-expanded={open}
          aria-autocomplete="list"
        />
      </label>
      {open && items.length > 0 && (
        <div className="suggest dl-suggest" onMouseDown={(e) => e.preventDefault()}>
          {sections.map((s) => (
            <section key={s.title}>
              <div className="suggest-title">{s.title}</div>
              <ul>
                {s.items.map((it) => {
                  const i = items.indexOf(it);
                  return (
                    <li key={it.key}>
                      <button className={i === active ? 'active' : ''} onMouseEnter={() => setActive(i)} onClick={it.run}>
                        {it.icon}
                        <span className="suggest-text">{it.text}</span>
                        {it.right && <span className="suggest-count">{it.right}</span>}
                      </button>
                    </li>
                  );
                })}
              </ul>
            </section>
          ))}
          <div className="suggest-footer">
            <span>
              Author <code>author:&quot;Jane Doe&quot;</code>
            </span>
            <span>
              Starred <code>favorites:true</code>
            </span>
            <span>
              Widget <code>widgets.title:latency</code>
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
