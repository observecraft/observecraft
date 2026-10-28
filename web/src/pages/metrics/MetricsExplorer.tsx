import { useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Plus, Search, Sigma, X } from 'lucide-react';
import type { MetricCatalogEntry } from '../../api/dashboards/types';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { expandFormula } from '../../lib/metricFormula';
import { usePopover } from '../../components/ui/Dropdown';
import { useMetricCatalog, useTimeseriesData } from '../dashboards/queries';
import { Chart } from '../../components/charts/Chart';
import { splitSeries } from './split';
import { DashboardContext, formatValue, type DashboardContextValue } from '../dashboards/widgets/context';
import { TimeseriesWidget } from '../dashboards/widgets/MetricWidgets';
import { KIND_LABEL } from './MetricsLayout';

export const AGGS = ['avg', 'sum', 'min', 'max'] as const;
export type Agg = (typeof AGGS)[number];

export interface Row {
  agg: Agg;
  metric: string;
  /** `env:prod, service:api-gateway` */
  from: string;
  /** `service, host` */
  by: string;
}

export const LETTERS = 'abcdefghij';

/** Row → query text: `avg:metric{env:prod,service:x} by {host}`. */
export function toQuery(r: Row): string {
  const from = r.from
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
    .join(',');
  const by = r.by
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
    .join(',');
  return `${r.agg}:${r.metric}{${from || '*'}}${by ? ` by {${by}}` : ''}`;
}

/** Metric name picker with search over the catalog. */
export function MetricPicker({ value, catalog, onChange }: { value: string; catalog: MetricCatalogEntry[]; onChange: (m: string) => void }) {
  const { open, setOpen, ref } = usePopover();
  const [text, setText] = useState('');
  const t = text.trim().toLowerCase();
  const shown = catalog.filter((m) => !t || m.name.toLowerCase().includes(t)).slice(0, 200);
  return (
    <div className="dd mx-picker" ref={ref}>
      <button type="button" className="mx-picker-btn" onClick={() => setOpen((o) => !o)}>
        {value || <span className="muted">Select a metric</span>}
      </button>
      {open && (
        <div className="dd-menu dd-left mx-picker-menu">
          <label className="mx-picker-search">
            <Search size={14} />
            <input autoFocus placeholder="Search metrics" value={text} onChange={(e) => setText(e.target.value)} />
          </label>
          <ul>
            {shown.map((m) => (
              <li key={m.name}>
                <button
                  type="button"
                  onClick={() => {
                    onChange(m.name);
                    setOpen(false);
                  }}
                >
                  <span className="mx-picker-name">{m.name}</span>
                  <span className="mx-kind">{KIND_LABEL[m.kind ?? 'gauge'] ?? m.kind}</span>
                </button>
              </li>
            ))}
            {shown.length === 0 && <li className="muted mx-picker-empty">No metric matches.</li>}
          </ul>
        </div>
      )}
    </div>
  );
}

/** Metrics → Explorer: build queries row by row and graph them over the page's time range. */
export function MetricsExplorer() {
  const [params, setParams] = useSearchParams();
  const { range } = useTime();
  const catalog = useMetricCatalog();
  // Starts empty, as Datadog's does; the first query lives in the URL so a graph can be shared or reloaded.
  const [rows, setRows] = useState<Row[]>(() => [{ agg: (params.get('agg') as Agg | null) ?? 'avg', metric: params.get('metric') ?? '', from: params.get('from') ?? '', by: params.get('by') ?? '' }]);
  useEffect(() => {
    const first = rows[0];
    setParams(
      (p) => {
        const next = new URLSearchParams(p);
        for (const [k, v] of Object.entries({ metric: first.metric, from: first.from, by: first.by, agg: first.agg === 'avg' ? '' : first.agg })) {
          if (v) next.set(k, v);
          else next.delete(k);
        }
        return next;
      },
      { replace: true },
    );
  }, [rows, setParams]);
  const update = (i: number, patch: Partial<Row>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const queries = rows.filter((r) => r.metric).map(toQuery);
  // A formula over the query letters ("a / b * 100") is graphed in place of the queries.
  const [formula, setFormula] = useState<string | null>(null);
  const byLetter = Object.fromEntries(rows.flatMap((r, i) => (r.metric ? [[LETTERS[i], toQuery(r)]] : [])));
  const expanded = formula?.trim() ? expandFormula(formula, byLetter) : null;
  const graphed = expanded ? [expanded] : queries;
  // Split graph: one small chart per group, for a single grouped query.
  const [split, setSplit] = useState(false);
  const canSplit = graphed.length === 1 && (Boolean(expanded) || Boolean(rows.find((r) => r.metric)?.by.trim()));
  const ctx = useMemo<DashboardContextValue>(
    () => ({ range, resolve: (q) => q, resolveSearch: (q) => q, canEdit: false, onEdit: () => {}, onClone: () => {}, onDelete: () => {}, onFullscreen: () => {}, onGroupLayout: () => {} }),
    [range],
  );
  const entry = (name: string) => catalog.data?.find((m) => m.name === name);
  return (
    <div className="mx-page">
      <div className="section-header">
        <div className="section-header-left">
          <h2 className="mx-title">Graph your metrics</h2>
        </div>
        <TimePicker />
      </div>
      <div className="mx-rows">
        {rows.map((r, i) => {
          const m = entry(r.metric);
          return (
            <div key={i} className="mx-row">
              <span className="mx-letter">{LETTERS[i]}</span>
              <select className="mx-agg" value={r.agg} onChange={(e) => update(i, { agg: e.target.value as Agg })} title="Space aggregation: how series are combined">
                {AGGS.map((a) => (
                  <option key={a} value={a}>
                    {a}
                  </option>
                ))}
              </select>
              <MetricPicker value={r.metric} catalog={catalog.data ?? []} onChange={(metric) => update(i, { metric })} />
              <label className="mx-field">
                from
                <input placeholder="everywhere (e.g. env:prod, service:api-gateway)" value={r.from} onChange={(e) => update(i, { from: e.target.value })} list={`mx-tags-${i}`} />
              </label>
              <label className="mx-field">
                by
                <input placeholder="(everything) e.g. service" value={r.by} onChange={(e) => update(i, { by: e.target.value })} list={`mx-tags-${i}`} />
              </label>
              <datalist id={`mx-tags-${i}`}>
                {(m?.tags ?? []).map((t) => (
                  <option key={t} value={t} />
                ))}
              </datalist>
              {rows.length > 1 && (
                <button className="icon-plain" title="Remove query" onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}>
                  <X size={16} />
                </button>
              )}
            </div>
          );
        })}
        {formula !== null && (
          <div className="mx-row">
            <span className="mx-letter mx-formula-letter">Σ</span>
            <input className="mx-formula" value={formula} onChange={(e) => setFormula(e.target.value)} placeholder="a / b * 100" aria-label="Formula" spellCheck={false} />
            {formula.trim() && !expanded && <span className="form-error">Use the query letters, numbers, + - * / and parentheses</span>}
            <button className="icon-plain" title="Remove formula" onClick={() => setFormula(null)}>
              <X size={16} />
            </button>
          </div>
        )}
        <div className="mx-row-actions">
          {rows.length < LETTERS.length && (
            <button className="btn btn-sm" onClick={() => setRows((rs) => [...rs, { agg: 'avg', metric: '', from: '', by: '' }])}>
              <Plus size={14} /> Add Query
            </button>
          )}
          {formula === null && (
            <button className="btn btn-sm" onClick={() => setFormula(rows.length > 1 ? 'a / b' : 'a * 1')}>
              <Sigma size={14} /> Add Formula
            </button>
          )}
          {graphed.map((q) => (
            <code key={q} className="mx-query">
              {q}
            </code>
          ))}
          {canSplit && (
            <label className="mx-split-toggle" title="One graph per group">
              <input type="checkbox" className="checkbox" checked={split} onChange={(e) => setSplit(e.target.checked)} /> Split graph
            </label>
          )}
        </div>
      </div>
      <div className={`mx-chart${split && canSplit ? ' mx-chart-split' : ''}`}>
        {graphed.length === 0 ? (
          <div className="wd-state">Pick a metric to graph it.</div>
        ) : (
          <DashboardContext.Provider value={ctx}>
            {split && canSplit ? <SplitGraph query={graphed[0]} /> : <TimeseriesWidget def={{ type: 'timeseries', display: 'line', requests: graphed.map((query) => ({ query })) }} />}
          </DashboardContext.Provider>
        )}
      </div>
      {rows.map((r) => entry(r.metric)).filter(Boolean).length > 0 && (
        <div className="mx-about">
          {rows.map((r, i) => {
            const m = entry(r.metric);
            return m ? (
              <p key={i}>
                <b>{m.name}</b> <span className="mx-kind">{KIND_LABEL[m.kind ?? 'gauge'] ?? m.kind}</span> {m.unit && <span className="muted">({m.unit})</span>} — {m.description || 'No description.'}
              </p>
            ) : null;
          })}
        </div>
      )}
    </div>
  );
}

/** Groups shown as small graphs at most. */
const SPLIT_LIMIT = 24;

/** One small graph per group of the query, the busiest first. */
function SplitGraph({ query }: { query: string }) {
  const { range } = useTime();
  const q = useTimeseriesData([query], range);
  if (q.isPending) return <div className="wd-state">Loading…</div>;
  if (q.isError) return <div className="wd-state wd-error">{q.error.message}</div>;
  const { shown, hidden } = splitSeries(q.data, SPLIT_LIMIT);
  return (
    <>
      <div className="mx-split">
        {shown.map((s) => (
          <div key={s.name} className="mx-split-cell">
            <div className="mx-split-title" title={s.name}>
              {s.name}
            </div>
            <Chart
              height={120}
              option={{
                grid: { left: 40, right: 8, top: 8, bottom: 20 },
                tooltip: { trigger: 'axis', confine: true },
                xAxis: { type: 'time', axisLabel: { fontSize: 10, hideOverlap: true } },
                yAxis: { type: 'value', axisLabel: { fontSize: 10, formatter: (v: number) => formatValue(v, Math.abs(v) < 1 ? 2 : 0) } },
                series: [{ name: s.name, type: 'line', showSymbol: false, lineStyle: { width: 1.2 }, data: s.points }],
              }}
            />
          </div>
        ))}
      </div>
      {hidden > 0 && <p className="muted">{hidden} more groups not shown.</p>}
    </>
  );
}
