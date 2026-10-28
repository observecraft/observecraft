import type { NavItem } from './navMenus';

/** One result of the top-bar search: a page, a create action or an entity (dashboard, monitor, team, user). */
export interface SearchEntry {
  kind: 'page' | 'action' | 'dashboard' | 'monitor' | 'team' | 'user';
  name: string;
  /** Product area for pages, entity type or detail for entities. */
  category: string;
  path: string;
}

export type PageChip = 'actions' | 'settings';

/** A one-word link ("Settings", "Triggered") is named with its group ("Monitors Settings"); longer labels stand alone. */
function linkName(groupTitle: string, label: string): string {
  return label.includes(' ') ? label : `${groupTitle} ${label}`;
}

/** Every sidebar page: menu group titles first, then their links, then the item itself; a path appears once. */
export function buildPageIndex(sections: NavItem[][], footer: NavItem[]): SearchEntry[] {
  const out: SearchEntry[] = [];
  const seen = new Set<string>();
  const add = (e: SearchEntry) => {
    if (seen.has(e.path)) return;
    seen.add(e.path);
    out.push(e);
  };
  for (const item of [...sections.flat(), ...footer]) {
    for (const g of item.menu ?? []) {
      if (g.to) add({ kind: 'page', name: g.title, category: item.label, path: g.to });
      for (const l of g.columns.flat()) {
        if (l.external) continue;
        add({ kind: l.plus ? 'action' : 'page', name: linkName(g.title, l.label), category: g.title, path: l.to });
      }
    }
    add({ kind: 'page', name: item.label, category: item.label, path: item.to });
  }
  return out;
}

/** Lower is better; Infinity when the token matches nothing. */
function tokenScore(e: SearchEntry, token: string): number {
  const name = e.name.toLowerCase();
  const words = name.split(/[\s\-/]+/);
  if (name === token) return 0;
  if (words.includes(token)) return 1;
  if (name.startsWith(token)) return 2;
  if (words.some((w) => w.startsWith(token))) return 3;
  if (name.includes(token)) return 4;
  if (e.category.toLowerCase().includes(token)) return 5;
  if (e.path.toLowerCase().includes(token)) return 6;
  return Infinity;
}

/** Entities (dashboards, monitors, …) give way to product pages that match as well. */
const ENTITY_PENALTY = 0.5;

/** Entries matching every word of the query, best first; an empty query keeps the given order. */
export function rankEntries(entries: SearchEntry[], query: string, limit = 50): SearchEntry[] {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!tokens.length) return entries.slice(0, limit);
  return entries
    .map((e, i) => ({ e, i, score: tokens.reduce((sum, t) => sum + tokenScore(e, t), 0) + (e.kind === 'page' || e.kind === 'action' ? 0 : ENTITY_PENALTY) }))
    .filter((r) => Number.isFinite(r.score))
    .sort((a, b) => a.score - b.score || a.i - b.i)
    .slice(0, limit)
    .map((r) => r.e);
}

/** The page index narrowed by a filter chip (null: no chip). */
export function entriesForChip(entries: SearchEntry[], chip: PageChip | null): SearchEntry[] {
  if (chip === 'actions') return entries.filter((e) => e.kind === 'action');
  if (chip === 'settings') return entries.filter((e) => /settings/i.test(e.name) || e.path.includes('settings'));
  return entries;
}
