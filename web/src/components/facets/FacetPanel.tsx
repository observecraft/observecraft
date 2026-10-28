import { useMemo, useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { BadgeCheck, ChevronDown, ChevronRight, Filter, Pencil, Plus, Search, X } from 'lucide-react';
import type { Facet } from '../../api/types';
import { clearKey, isValueChecked, onlyValue, selectedValues, toggleChecked, toggleValue, type Term } from '../../lib/searchQuery';
import { STATUS_COLORS } from '../../theme/chartTheme';
import './facets.css';

interface Props {
  facets: Facet[];
  /** Parsed current query — the panel derives checkbox state from it and edits it. */
  terms: Term[];
  onChange: (terms: Term[]) => void;
  /**
   * include: values start unchecked; checking filters to them (Log Explorer).
   * exclude: values start checked; unchecking excludes, clicking a label keeps only it (Error Tracking).
   */
  mode?: 'include' | 'exclude';
  /** Per-facet "Filter values" box. */
  valueSearch?: boolean;
  valueLimit?: number;
  defaultOpen?: string[];
  verified?: string[];
  /** Total facets available to add, for "Showing N of M …". Defaults to facets.length. */
  totalAvailable?: number;
  noun?: string;
  renderValueIcon?: (key: string, value: string) => ReactNode;
  /** Display label of a value (default: the value, statuses capitalized). */
  valueLabel?: (key: string, value: string) => string;
  /** Extra controls above the facet search (e.g. Teams, toggles). */
  header?: ReactNode;
  /** "+ Add": the fields that can become facets, and what adding one does. No button without it. Standard attributes are listed first. */
  addable?: { load: () => Promise<{ key: string; count: number; standard?: boolean }[]>; onAdd: (key: string) => void };
}

/** The "+ Add" list: fields of the current results, filtered as you type. */
function AddFacet({ addable, present, onClose }: { addable: NonNullable<Props['addable']>; present: Set<string>; onClose: () => void }) {
  const [filter, setFilter] = useState('');
  const fields = useQuery({ queryKey: ['facets', 'addable', addable.load], queryFn: addable.load });
  const shown = (fields.data ?? [])
    .filter((f) => !present.has(f.key) && f.key.toLowerCase().includes(filter.toLowerCase()))
    .sort((a, b) => Number(Boolean(b.standard)) - Number(Boolean(a.standard)))
    .slice(0, 50);
  return (
    <div className="facet-add-menu">
      <input autoFocus placeholder="Filter fields, e.g. kube_node or @http.url" value={filter} onChange={(e) => setFilter(e.target.value)} onKeyDown={(e) => e.key === 'Escape' && onClose()} />
      <div className="facet-add-list">
        {fields.isPending && <div className="muted-text">Loading fields…</div>}
        {fields.error && <div className="muted-text">Fields could not be loaded: {fields.error.message}</div>}
        {!fields.isPending && !shown.length && !fields.error && <div className="muted-text">No other fields in these logs.</div>}
        {shown.map((f) => (
          <button
            key={f.key}
            onClick={() => {
              addable.onAdd(f.key);
              onClose();
            }}
          >
            <span className="mono">{f.key}</span>
            {f.standard && (
              <span className="facet-standard" title="A standard attribute of your organization">
                standard
              </span>
            )}
            <span className="muted-text">{f.count.toLocaleString()}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function label(key: string, value: string): string {
  return key === 'status' ? value.charAt(0).toUpperCase() + value.slice(1) : value;
}

function toggleSet(set: Set<string>, key: string): Set<string> {
  const next = new Set(set);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  return next;
}

export function FacetPanel({
  facets,
  terms,
  onChange,
  mode = 'include',
  valueSearch = false,
  valueLimit = 10,
  defaultOpen = ['service', 'status'],
  verified = ['source', 'host', 'service', 'status'],
  totalAvailable,
  noun = '',
  renderValueIcon,
  valueLabel = label,
  header,
  addable,
}: Props) {
  const [adding, setAdding] = useState(false);
  const [search, setSearch] = useState('');
  const [open, setOpen] = useState<Set<string>>(() => new Set(defaultOpen));
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [valueFilter, setValueFilter] = useState<Record<string, string>>({});
  const [closedGroups, setClosedGroups] = useState<Set<string>>(new Set(['HTTP']));

  const visible = facets.filter((f) => f.label.toLowerCase().includes(search.toLowerCase()));
  const groups = useMemo(() => {
    const m = new Map<string, Facet[]>();
    for (const f of visible) m.set(f.group, [...(m.get(f.group) ?? []), f]);
    return [...m.entries()];
  }, [visible]);
  const selected = useMemo(() => selectedValues(terms), [terms]);
  const verifiedSet = useMemo(() => new Set(verified), [verified]);

  const checked = (key: string, value: string) => (mode === 'exclude' ? isValueChecked(terms, key, value) : (selected[key] ?? []).includes(value));
  const toggle = (key: string, value: string) => onChange(mode === 'exclude' ? toggleChecked(terms, key, value) : toggleValue(terms, key, value));

  const renderFacet = (f: Facet) => {
    const isOpen = open.has(f.key);
    const filter = (valueFilter[f.key] ?? '').toLowerCase();
    const values = f.values.filter((v) => v.value.toLowerCase().includes(filter));
    const shown = expanded.has(f.key) ? values : values.slice(0, valueLimit);
    return (
      <div className={`facet${mode === 'exclude' ? ' facet-plain' : ''}`} key={f.key}>
        <div className="facet-head">
          <button className="facet-title" onClick={() => setOpen((s) => toggleSet(s, f.key))}>
            {isOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            {f.label}
            {verifiedSet.has(f.key) && <BadgeCheck size={15} className="facet-verified" />}
          </button>
          {isOpen && mode === 'include' && (
            <span className="facet-actions">
              <button className="icon-btn" title="Filter values">
                <Filter size={11} />
              </button>
              <button className="icon-btn" title="Clear" onClick={() => onChange(clearKey(terms, f.key))}>
                <X size={11} />
              </button>
            </span>
          )}
        </div>
        {isOpen && (
          <>
            {valueSearch && (
              <label className="facet-value-search">
                <Search size={13} />
                <input placeholder="Filter values" value={valueFilter[f.key] ?? ''} onChange={(e) => setValueFilter((m) => ({ ...m, [f.key]: e.target.value }))} />
              </label>
            )}
            <ul className="facet-values">
              {shown.map((v) => (
                <li key={v.value}>
                  <label>
                    <input type="checkbox" className="checkbox" checked={checked(f.key, v.value)} onChange={() => toggle(f.key, v.value)} />
                    {f.key === 'status' && v.value in STATUS_COLORS && <span className="status-bar" style={{ background: `var(${STATUS_COLORS[v.value as keyof typeof STATUS_COLORS]})` }} />}
                    {renderValueIcon?.(f.key, v.value)}
                    <span
                      className="facet-value"
                      onClick={
                        mode === 'exclude'
                          ? (e) => {
                              e.preventDefault();
                              onChange(onlyValue(terms, f.key, v.value));
                            }
                          : undefined
                      }
                    >
                      {valueLabel(f.key, v.value)}
                    </span>
                    <span className="facet-count">{v.count > 0 ? v.count.toLocaleString() : '-'}</span>
                  </label>
                </li>
              ))}
            </ul>
            {values.length > valueLimit && (
              <button className="facet-more" onClick={() => setExpanded((s) => toggleSet(s, f.key))}>
                {expanded.has(f.key) ? 'View fewer values' : 'View more values'}
              </button>
            )}
          </>
        )}
      </div>
    );
  };

  return (
    <aside className="facets">
      {header}
      <label className="facet-search">
        <Search size={14} />
        <input placeholder="Search facets" value={search} onChange={(e) => setSearch(e.target.value)} />
      </label>
      <div className="facet-meta">
        <Pencil size={13} />
        <span>
          Showing {visible.length} of {(totalAvailable ?? facets.length).toLocaleString()}
          {noun && ` ${noun}`}
        </span>
        {addable && (
          <button className="facet-add" onClick={() => setAdding((a) => !a)}>
            <Plus size={15} /> Add
          </button>
        )}
      </div>
      {addable && adding && <AddFacet addable={addable} present={new Set(facets.map((f) => f.key))} onClose={() => setAdding(false)} />}

      {groups.map(([group, list]) => {
        if (!group) return list.map(renderFacet);
        const groupOpen = !closedGroups.has(group);
        return (
          <div key={group}>
            <button className="facet-group" onClick={() => setClosedGroups((s) => toggleSet(s, group))}>
              {groupOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              {group}
            </button>
            {groupOpen && list.map(renderFacet)}
          </div>
        );
      })}
    </aside>
  );
}
