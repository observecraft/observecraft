// The dashboard list's search syntax, read as the server reads it (routes/dashboards.rs `parse_search`): `author:`,
// `team:`, `favorites:` and `widgets.title:` filters (values may be quoted), the rest matched against titles and
// authors. Other `key:value` words and filters without a value are text.

export interface DashboardSearchTerms {
  text: string;
  authors: string[];
  teams: string[];
  favorites: boolean | null;
  widgetTitles: string[];
}

/** The filters the search box suggests, in its dropdown's order. */
export const SEARCH_FILTERS = ['author', 'favorites', 'team', 'widgets.title'] as const;

export function parseDashboardSearch(query: string): DashboardSearchTerms {
  const words: string[] = [];
  let word = '';
  let quoted = false;
  for (const c of query) {
    if (c === '"') quoted = !quoted;
    else if (/\s/.test(c) && !quoted) {
      if (word) words.push(word);
      word = '';
    } else word += c;
  }
  if (word) words.push(word);
  const terms: DashboardSearchTerms = { text: '', authors: [], teams: [], favorites: null, widgetTitles: [] };
  const text: string[] = [];
  for (const w of words) {
    const i = w.indexOf(':');
    const key = i < 0 ? '' : w.slice(0, i).toLowerCase();
    const value = i < 0 ? '' : w.slice(i + 1);
    if (!value) {
      if (!(SEARCH_FILTERS as readonly string[]).includes(key)) text.push(w);
    } else if (key === 'author') terms.authors.push(value);
    else if (key === 'team') terms.teams.push(value);
    else if (key === 'favorites') terms.favorites = value.toLowerCase() === 'true';
    else if (key === 'widgets.title') terms.widgetTitles.push(value);
    else text.push(w);
  }
  terms.text = text.join(' ');
  return terms;
}

export interface RecentSearch {
  query: string;
  at: number;
}

/** `list` with `query` searched at `at` first: blanks skipped, an earlier same search (ignoring case) dropped, five kept. */
export function addRecentSearch(list: RecentSearch[], query: string, at: number): RecentSearch[] {
  const q = query.trim();
  if (!q) return list;
  return [{ query: q, at }, ...list.filter((r) => r.query.toLowerCase() !== q.toLowerCase())].slice(0, 5);
}
