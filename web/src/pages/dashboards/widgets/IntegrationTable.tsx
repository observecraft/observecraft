import { useMemo, useState } from 'react';
import { ArrowDownWideNarrow, ArrowUpNarrowWide, Search } from 'lucide-react';
import type { IntegrationWidgetDefinition } from '../../../api/integrations/evaluate';
import type { PanelSeries } from '../../../api/integrations/panels';
import { formatMetricValue } from '../../../api/integrations/units';
import { fills, formatStyle, matchFormat, type ConditionalFormat } from './conditionalFormats';

interface TableFormula {
  formula: string;
  alias?: string;
  cell_display_mode?: string;
  conditional_formats?: ConditionalFormat[];
}

interface TableRequest {
  queries?: { query: string }[];
  formulas?: TableFormula[];
  sort?: { count?: number; order_by?: { type: string; index?: number; order?: string }[] };
}

/** `kube_cluster_name:shop-prod, env:prod` → its tags; `*` (no group by) has none. */
export function groupTags(group: string): Map<string, string> {
  if (group === '*') return new Map();
  return new Map(
    group.split(', ').map((pair) => {
      const i = pair.indexOf(':');
      return i < 0 ? [pair, ''] : [pair.slice(0, i), pair.slice(i + 1)];
    }),
  );
}

type Sort = { column: string; desc: boolean };

/** A query_table: one column per group-by tag, then one per formula, as the upstream widget lays it out. */
export function IntegrationTable({ def, series }: { def: IntegrationWidgetDefinition; series: PanelSeries[] }) {
  const request = (def.requests[0] ?? {}) as TableRequest;
  const formulas = request.formulas ?? [];
  const formulaColumns = useMemo(() => {
    const named = formulas.map((f) => ({ key: f.alias ?? f.formula, label: f.alias ?? f.formula, formula: f as TableFormula | undefined }));
    // Series of requests without formulas (or columns not declared) still get a column.
    for (const s of series) {
      const key = s.column ?? s.name;
      if (!named.some((c) => c.key === key)) named.push({ key, label: key, formula: undefined });
    }
    return named;
  }, [formulas, series]);
  const units = new Map(series.map((s) => [s.column ?? s.name, s.unit]));
  const tagColumns = useMemo(() => {
    const declared = /\bby\s*\{([^}]*)\}/.exec(request.queries?.[0]?.query ?? '')?.[1];
    const keys = declared ? declared.split(',').map((k) => k.trim()) : [];
    for (const s of series) for (const key of groupTags(s.group ?? s.name).keys()) if (!keys.includes(key)) keys.push(key);
    return keys.filter(Boolean);
  }, [request.queries, series]);
  const rows = useMemo(() => {
    const byGroup = new Map<string, { tags: Map<string, string>; values: Map<string, number> }>();
    for (const s of series) {
      const group = s.group ?? s.name;
      const row = byGroup.get(group) ?? { tags: groupTags(group), values: new Map<string, number>() };
      const value = s.points.at(-1)?.[1];
      if (value !== undefined && Number.isFinite(value)) row.values.set(s.column ?? s.name, value);
      byGroup.set(group, row);
    }
    return [...byGroup.values()];
  }, [series]);
  const initial = request.sort?.order_by?.find((o) => o.type === 'formula');
  const [sort, setSort] = useState<Sort | null>(null);
  const active: Sort | null = sort ?? (formulaColumns[initial?.index ?? 0] ? { column: formulaColumns[initial?.index ?? 0].key, desc: initial?.order !== 'asc' } : null);
  const [search, setSearch] = useState('');
  const searchable = (def.has_search_bar === 'always' || def.has_search_bar === 'auto') && tagColumns.length > 0;
  const needle = search.trim().toLowerCase();
  const visible = rows
    .filter((r) => !needle || [...r.tags.values()].some((v) => v.toLowerCase().includes(needle)))
    .sort((a, b) => {
      if (!active) return 0;
      const sign = active.desc ? -1 : 1;
      if (tagColumns.includes(active.column)) return sign * (a.tags.get(active.column) ?? '').localeCompare(b.tags.get(active.column) ?? '');
      const [x, y] = [a.values.get(active.column), b.values.get(active.column)];
      // Rows without a value stay at the bottom either way.
      if (x === undefined || y === undefined) return x === undefined ? (y === undefined ? 0 : 1) : -1;
      return sign * (x - y);
    })
    .slice(0, request.sort?.count ?? 500);
  const max = new Map(formulaColumns.map((c) => [c.key, Math.max(0, ...rows.map((r) => Math.abs(r.values.get(c.key) ?? 0)))]));
  const header = (key: string, label: string, numeric: boolean) => (
    <th key={key} className={numeric ? 'num' : undefined} title={label} onClick={() => setSort({ column: key, desc: active?.column === key ? !active.desc : numeric })}>
      {active?.column === key && (active.desc ? <ArrowDownWideNarrow size={13} className="iw-qt-sort" /> : <ArrowUpNarrowWide size={13} className="iw-qt-sort" />)}
      {label}
    </th>
  );
  return (
    <div className="wt iw-qt">
      {searchable && (
        <label className="wt-search wf-nodrag">
          <Search size={14} />
          <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Search" />
        </label>
      )}
      <div className="wt-scroll">
        <table>
          <thead>
            <tr>
              {tagColumns.map((key) => header(key, key, false))}
              {formulaColumns.map((c) => header(c.key, c.label, true))}
            </tr>
          </thead>
          <tbody>
            {visible.map((row, i) => (
              <tr key={i}>
                {tagColumns.map((key) => (
                  <td key={key} title={row.tags.get(key) ?? 'N/A'}>
                    {row.tags.get(key) || 'N/A'}
                  </td>
                ))}
                {formulaColumns.map((c) => {
                  const value = row.values.get(c.key);
                  const format = matchFormat(value, c.formula?.conditional_formats);
                  const style = format ? formatStyle(format.palette) : undefined;
                  const bar = c.formula?.cell_display_mode === 'bar' && value !== undefined;
                  return (
                    <td key={c.key} className={`num${format && fills(format.palette) ? ' iw-qt-filled' : ''}`} style={style}>
                      {value === undefined ? '—' : formatMetricValue(value, units.get(c.key), 2)}
                      {bar && (
                        <span className="iw-qt-bar">
                          <span style={{ width: `${max.get(c.key) ? (Math.abs(value) / max.get(c.key)!) * 100 : 0}%` }} />
                        </span>
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
