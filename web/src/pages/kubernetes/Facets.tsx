// The facet panel of Remediation and Live Rollouts: per facet, its values with counts as checkboxes.
import { useState } from 'react';
import { ChevronDown, ChevronRight, Search } from 'lucide-react';
import { facetValues } from './autoscaling/model';

export interface FacetDef<T> {
  key: string;
  title: string;
  value: (item: T) => string | null | undefined;
  /** Display text of a value (issue labels, statuses). */
  label?: (value: string) => string;
}

export function Facets<T>({ items, facets, selected, onChange }: { items: T[]; facets: FacetDef<T>[]; selected: Record<string, string[]>; onChange: (next: Record<string, string[]>) => void }) {
  const [search, setSearch] = useState('');
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const q = search.trim().toLowerCase();
  return (
    <aside className="as-facets">
      <label className="as-search">
        <Search size={15} />
        <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Search facets" />
      </label>
      {facets
        .filter((f) => !q || f.title.toLowerCase().includes(q))
        .map((f) => {
          const open = !closed.has(f.key);
          const values = facetValues(items, f.value);
          const chosen = selected[f.key] ?? [];
          const toggle = (v: string, on: boolean) => onChange({ ...selected, [f.key]: on ? [...chosen, v] : chosen.filter((x) => x !== v) });
          return (
            <div key={f.key} className="as-facet">
              <button className="as-facet-head" onClick={() => setClosed((c) => (open ? new Set(c).add(f.key) : new Set([...c].filter((k) => k !== f.key))))}>
                {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />} {f.title}
              </button>
              {open &&
                (values.length ? (
                  values.map((v) => (
                    <label key={v.value} className="as-facet-value">
                      <input type="checkbox" checked={chosen.includes(v.value)} onChange={(e) => toggle(v.value, e.target.checked)} />
                      <span>{f.label ? f.label(v.value) : v.value}</span>
                      <span className="muted">{v.count}</span>
                    </label>
                  ))
                ) : (
                  <span className="muted as-facet-empty">No values</span>
                ))}
            </div>
          );
        })}
    </aside>
  );
}
