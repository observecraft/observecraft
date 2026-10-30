import { useMemo, useState, type ReactNode } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { ChevronsDownUp, ChevronsUpDown, RotateCcw, X } from 'lucide-react';
import type { Widget } from '../../../api/dashboards/types';
import { aggregate, normalizeRequest, type IntegrationRequest, type IntegrationWidgetDefinition } from '../../../api/integrations/evaluate';
import { queryIntegrationPanel, type PanelSeries } from '../../../api/integrations/panels';
import { formatMetricValue, formatUnitValue, presentationUnit } from '../../../api/integrations/units';
import type { TimeRange } from '../../../api/types';
import { Chart } from '../../../components/charts/Chart';
import { TimePicker } from '../../../components/shell/PageHeader';
import { Select } from '../../../components/ui/Dropdown';
import { palette } from '../../../theme/chartTheme';
import { QueryBuilder } from '../editor/QueryBuilder';
import { useDashboardContext } from './context';
import { groupTags } from './IntegrationTable';
import { useWidgetRange } from './IntegrationWidget';
import { WidgetState } from './MetricWidgets';
import { WidgetContent, WidgetShare } from './WidgetFrame';

/** Tags a row shows before "+N". */
const TAGS_SHOWN = 2;
/** Small charts the split view draws at most. */
const SPLIT_LIMIT = 24;

type Tab = 'inspect' | 'split' | 'compare';
const TABS: [Tab, string][] = [
  ['inspect', 'Inspect'],
  ['split', 'Split Graph'],
  ['compare', 'Compare Time'],
];
const COMPARE: [string, number][] = [
  ['Hour before', 3600],
  ['Day before', 86400],
  ['Week before', 604800],
  ['Month before', 2592000],
];

/** The widget's requests with the queries and formulas edited here (local only: nothing is saved). */
function withEdits(def: IntegrationWidgetDefinition, queries: Record<string, string>, formulas: Record<string, string>): IntegrationWidgetDefinition {
  if (!Object.keys(queries).length && !Object.keys(formulas).length) return def;
  return {
    ...def,
    requests: def.requests.map((r, ri) => {
      const n = normalizeRequest(r);
      return {
        ...r,
        q: undefined,
        queries: n.queries.map((q) => ({ ...q, query: queries[`${ri}:${q.name}`] ?? q.query })),
        formulas: n.formulas?.map((f, fi) => ({ ...f, formula: formulas[`${ri}:${fi}`] ?? f.formula })),
      } as IntegrationRequest;
    }),
  };
}

function useSeries(def: IntegrationWidgetDefinition, range: TimeRange, enabled = true) {
  const { resolve } = useDashboardContext();
  const identity = JSON.stringify(def).replace(/\$[a-zA-Z_]\w*/g, (v) => resolve(`avg:scope{${v}}`));
  return useQuery({
    queryKey: ['integration-panel', 'inspect', identity, range],
    queryFn: () => queryIntegrationPanel({ ...def, type: 'timeseries' }, resolve, range),
    enabled,
    placeholderData: keepPreviousData,
    retry: false,
  });
}

function lineOption(series: PanelSeries[], options: { hovered?: string | null; dashed?: Set<string>; colorOf?: (s: PanelSeries, i: number) => string } = {}): EChartsOption {
  const colors = palette();
  const axis = presentationUnit(series[0]?.unit);
  return {
    grid: { left: 8, right: 16, top: 12, bottom: 26, containLabel: true },
    tooltip: { trigger: 'axis', confine: true },
    xAxis: { type: 'time', axisLabel: { hideOverlap: true } },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => formatUnitValue(v, axis.unit, 2) } },
    series: series.map((s, i) => ({
      name: s.name,
      type: 'line',
      showSymbol: false,
      color: options.colorOf?.(s, i) ?? colors[i % colors.length],
      lineStyle: {
        width: options.hovered === s.name ? 2.5 : 1.2,
        opacity: options.hovered && options.hovered !== s.name ? 0.15 : 1,
        type: options.dashed?.has(s.name) ? 'dashed' : 'solid',
      },
      z: options.hovered === s.name ? 10 : 2,
      tooltip: { valueFormatter: (v: unknown) => formatUnitValue(Number(v), axis.unit) },
      data: s.points.map(([t, v]) => [t, v * axis.scale]),
    })),
  };
}

const fmt = (v: number | undefined, unit?: string) => (v === undefined ? '—' : formatMetricValue(v, unit, 2));
const tagValues = (s: PanelSeries) => [...groupTags(s.group ?? s.name).values()].join(', ') || s.name;

/**
 * A metric widget opened in full screen: its queries (which can be tried out here without saving), the
 * widget with the dashboard's filters, and its series over time — inspected one by one, split into a graph
 * per group, or compared with an earlier period.
 */
export function IntegrationInspect({ widget, onClose, filters }: { widget: Widget; onClose: () => void; filters?: ReactNode }) {
  const original = (widget.definition as { original: IntegrationWidgetDefinition }).original;
  const { range: dashboardRange } = useDashboardContext();
  const range = useWidgetRange(original, dashboardRange);
  const [tab, setTab] = useState<Tab>('inspect');
  const [expanded, setExpanded] = useState(false);
  const [queryEdits, setQueryEdits] = useState<Record<string, string>>({});
  const [formulaEdits, setFormulaEdits] = useState<Record<string, string>>({});
  const [hovered, setHovered] = useState<string | null>(null);
  const [shift, setShift] = useState(86400);
  const def = useMemo(() => withEdits(original, queryEdits, formulaEdits), [original, queryEdits, formulaEdits]);
  const edited = def !== original;
  const requests = def.requests.map((r) => normalizeRequest(r));
  const data = useSeries(def, range);
  const shifted = useMemo(() => ({ from: range.from - shift * 1000, to: range.to - shift * 1000 }), [range, shift]);
  const before = useSeries(def, shifted, tab === 'compare');
  const series = data.data ?? [];
  const colors = palette();
  const rows = series.map((s, i) => {
    const values = s.points.map((p) => p[1]);
    return {
      name: s.name,
      color: colors[i % colors.length],
      unit: s.unit,
      tags: [...groupTags(s.group ?? s.name)],
      avg: aggregate(values, 'avg'),
      min: aggregate(values, 'min'),
      max: aggregate(values, 'max'),
      sum: aggregate(values, 'sum'),
      value: values.at(-1),
    };
  });
  const averages = rows.map((r) => r.avg).filter((v): v is number => v !== undefined);
  const unit = series[0]?.unit;
  // The group-by tags of the first query, filled in with the hovered series' values.
  const groupBy =
    /\bby\s*\{([^}]*)\}/
      .exec(requests[0]?.queries[0]?.query ?? '')?.[1]
      .split(',')
      .map((k) => k.trim()) ?? [];
  const hoveredTags = new Map(rows.find((r) => r.name === hovered)?.tags ?? []);
  const letter = (i: number) => String.fromCharCode(97 + i);
  let index = 0;

  const compareSeries: PanelSeries[] = [
    ...series,
    ...(before.data ?? []).map((s) => ({
      ...s,
      name: `${s.name} (${COMPARE.find(([, v]) => v === shift)?.[0].toLowerCase()})`,
      points: s.points.map(([t, v]) => [t + shift * 1000, v] as [number, number]),
    })),
  ];
  const compareColor = (_s: PanelSeries, i: number) => colors[(i % Math.max(1, series.length)) % colors.length];

  return (
    <div className="iw-inspect">
      <header className="iw-inspect-header">
        <nav className="iw-inspect-tabs">
          {TABS.map(([key, label]) => (
            <button key={key} type="button" className={tab === key ? 'active' : undefined} onClick={() => setTab(key)}>
              {label}
            </button>
          ))}
        </nav>
        <TimePicker />
        <button type="button" className="wf-menu-btn" title="Close" onClick={onClose}>
          <X size={18} />
        </button>
      </header>

      <div className="iw-inspect-queries">
        <div className="iw-inspect-query-bar">
          <button type="button" className="link-btn" onClick={() => setExpanded((e) => !e)}>
            {expanded ? <ChevronsDownUp size={15} /> : <ChevronsUpDown size={15} />} {expanded ? 'Collapse Query' : 'Expand Query'}
          </button>
          {expanded ? (
            <span className="muted-text">Experiment with your query. Changes stay in this view; clone the dashboard to keep them.</span>
          ) : (
            <span className="mono iw-inspect-query-text">{requests.flatMap((r) => r.queries.map((q) => q.query)).join('  ·  ')}</span>
          )}
          {edited && (
            <button
              type="button"
              className="link-btn"
              onClick={() => {
                setQueryEdits({});
                setFormulaEdits({});
              }}
            >
              <RotateCcw size={14} /> Reset
            </button>
          )}
        </div>
        {expanded && (
          <div className="iw-inspect-editor">
            {requests.map((r, ri) => (
              <div key={ri}>
                {r.queries.map((q) => {
                  const i = index++;
                  return (
                    <div key={q.name} className="iw-inspect-editor-row">
                      <span className="iw-inspect-letter" title={q.name}>
                        {letter(i)}
                      </span>
                      {q.data_source === 'metrics' ? (
                        <QueryBuilder value={q.query} onChange={(v) => setQueryEdits((e) => ({ ...e, [`${ri}:${q.name}`]: v }))} />
                      ) : (
                        <span className="mono muted-text">{q.query}</span>
                      )}
                    </div>
                  );
                })}
                {r.formulas?.map((f, fi) => (
                  <div key={fi} className="iw-inspect-editor-row">
                    <span className="iw-inspect-letter iw-inspect-formula">ƒ</span>
                    <input className="input mono" value={f.formula} onChange={(e) => setFormulaEdits((x) => ({ ...x, [`${ri}:${fi}`]: e.target.value }))} />
                    {f.alias && <span className="muted-text">as {f.alias}</span>}
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>

      <div className={`iw-inspect-body${tab === 'inspect' ? '' : ' iw-inspect-single'}`}>
        {tab === 'inspect' && (
          <section className="iw-inspect-widget">
            <div className="iw-inspect-head">
              <h3>{original.title || widget.definition.title}</h3>
              <WidgetShare widget={widget} label="Share" />
            </div>
            {averages.length > 1 && (
              <p className="iw-inspect-summary">
                <strong>{averages.length}</strong> groups ranging from <strong>{fmt(Math.min(...averages), unit)}</strong> to <strong>{fmt(Math.max(...averages), unit)}</strong>.
              </p>
            )}
            {filters && <div className="iw-inspect-filters">{filters}</div>}
            <div className="iw-inspect-content">
              <WidgetContent def={{ ...widget.definition, original: def } as Widget['definition']} />
            </div>
          </section>
        )}
        <section className="iw-inspect-series">
          {tab === 'inspect' && (
            <div className="iw-inspect-head">
              <span className="iw-inspect-groups">
                {groupBy.map((k) => (
                  <span key={k} className="iw-ev-tag">
                    {k}: <b>{hoveredTags.get(k) ?? '*'}</b>
                  </span>
                ))}
                {!hovered && <span className="muted-text">Hover over a group for details</span>}
              </span>
              <WidgetShare widget={widget} label="Share" />
            </div>
          )}
          {tab === 'compare' && (
            <div className="iw-inspect-head">
              <span className="iw-inspect-groups">
                Compare with
                <Select value={String(shift)} options={COMPARE.map(([label, seconds]) => ({ value: String(seconds), label }))} onChange={(v) => setShift(Number(v))} />
                <span className="muted-text">dashed lines are the earlier period</span>
              </span>
            </div>
          )}
          <WidgetState loading={data.isPending} error={data.error} empty={!series.length}>
            {tab === 'inspect' && (
              <>
                <div className="iw-inspect-chart">
                  <Chart option={lineOption(series, { hovered })} height="100%" />
                </div>
                <div className="wt iw-inspect-table">
                  <div className="wt-scroll">
                    <table>
                      <thead>
                        <tr>
                          <th>Tags</th>
                          <th className="num">Avg</th>
                          <th className="num">Min</th>
                          <th className="num">Max</th>
                          <th className="num">Sum</th>
                          <th className="num">Value</th>
                        </tr>
                      </thead>
                      <tbody>
                        {rows.map((r) => (
                          <tr key={r.name} onMouseEnter={() => setHovered(r.name)} onMouseLeave={() => setHovered(null)} className={hovered === r.name ? 'selected' : undefined}>
                            <td>
                              <span className="iw-inspect-tags">
                                <span className="iw-inspect-swatch" style={{ background: r.color }} />
                                {r.tags.slice(0, TAGS_SHOWN).map(([k, v]) => (
                                  <span key={k} className="iw-ev-tag" title={`${k}:${v}`}>
                                    {k}:<b>{v}</b>
                                  </span>
                                ))}
                                {r.tags.length > TAGS_SHOWN && <span className="iw-ev-tag iw-ev-more">+{r.tags.length - TAGS_SHOWN}</span>}
                                {!r.tags.length && <span>{r.name}</span>}
                              </span>
                            </td>
                            <td className="num">{fmt(r.avg, r.unit)}</td>
                            <td className="num">{fmt(r.min, r.unit)}</td>
                            <td className="num">{fmt(r.max, r.unit)}</td>
                            <td className="num">{fmt(r.sum, r.unit)}</td>
                            <td className="num">{fmt(r.value, r.unit)}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              </>
            )}
            {tab === 'split' && (
              <div className="iw-inspect-split">
                {series.slice(0, SPLIT_LIMIT).map((s, i) => (
                  <div key={s.name} className="iw-inspect-split-cell">
                    <span className="iw-inspect-split-title" title={s.name}>
                      {tagValues(s)}
                    </span>
                    <Chart option={lineOption([s], { colorOf: () => colors[i % colors.length] })} height="100%" />
                  </div>
                ))}
                {series.length > SPLIT_LIMIT && (
                  <p className="muted-text">
                    Showing {SPLIT_LIMIT} of {series.length} groups.
                  </p>
                )}
              </div>
            )}
            {tab === 'compare' && (
              <div className="iw-inspect-compare">
                <Chart option={lineOption(compareSeries, { dashed: new Set(compareSeries.slice(series.length).map((s) => s.name)), colorOf: compareColor })} height="100%" />
              </div>
            )}
          </WidgetState>
        </section>
      </div>
    </div>
  );
}
