import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowLeftRight, ChevronsLeft, ChevronsRight, ChevronsUpDown, ExternalLink, ListFilter } from 'lucide-react';
import { flameGraph, profileOverview, type FlameGraph } from '../../../api/profiles/client';
import type { TimeRange } from '../../../api/types';
import { QueryBar } from '../../../components/query/QueryBar';
import { RELATIVE_RANGES } from '../../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { token } from '../../../theme/chartTheme';
import { FlameGraphView, valuesByKey, viewTree, type GroupBy, type ViewNode } from './FlameGraph';
import { MetricChart } from './ProfileExplorer';
import { averageLabel, chartLabel, fmtValue, frameLabel } from './format';

interface Side {
  durationMs: number;
  to: number;
  query: string;
}

function fmtRange(r: TimeRange): string {
  const f = (t: number) => new Date(t).toLocaleString('en-US', { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }).replace(' AM', ' am').replace(' PM', ' pm');
  return `${f(r.from)} – ${f(r.to)}`;
}

const rangeOf = (s: Side): TimeRange => ({ from: s.to - s.durationMs, to: s.to });

function SidePicker({ name, side, onChange, onOpen }: { name: 'A' | 'B'; side: Side; onChange: (s: Side) => void; onOpen: () => void }) {
  const r = rangeOf(side);
  return (
    <div className={`pf-side pf-side-${name}`}>
      <div className="pf-side-badge">{name}</div>
      <div className="pf-side-body">
        <div className="pf-side-time">
          <div className="pf-range">
            <select value={side.durationMs} onChange={(e) => onChange({ ...side, durationMs: Number(e.target.value) })}>
              {RELATIVE_RANGES.map((x) => (
                <option key={x.key} value={x.ms}>
                  {x.short}
                </option>
              ))}
            </select>
            <span>{fmtRange(r)}</span>
          </div>
          <div className="btn-group">
            <button className="btn" title="Earlier" onClick={() => onChange({ ...side, to: side.to - side.durationMs })}>
              <ChevronsLeft size={15} />
            </button>
            <button className="btn" title="Later" onClick={() => onChange({ ...side, to: Math.min(Date.now(), side.to + side.durationMs) })}>
              <ChevronsRight size={15} />
            </button>
          </div>
          <button className="btn pf-side-open" title="Open in the Explorer" onClick={onOpen}>
            <ExternalLink size={15} />
          </button>
        </div>
        <QueryBar value={side.query} onChange={(query) => onChange({ ...side, query })} placeholder="Filter your profiles" />
      </div>
    </div>
  );
}

/** Function self values of both sides, for the table. */
function DiffTable({ a, b }: { a: FlameGraph; b: FlameGraph }) {
  const unit = b.metric?.unit ?? 'count';
  const rows = useMemo(() => {
    const map = new Map<string, { name: string; file: string; a: number; b: number }>();
    for (const [fg, side] of [
      [a, 'a'],
      [b, 'b'],
    ] as const) {
      for (const f of fg.functions) {
        const k = `${f.name}@${f.file}`;
        const r = map.get(k) ?? { name: f.name, file: f.file, a: 0, b: 0 };
        r[side] += f.value * fg.scale;
        map.set(k, r);
      }
    }
    return [...map.values()].sort((x, y) => Math.abs(y.b - y.a) - Math.abs(x.b - x.a)).slice(0, 200);
  }, [a, b]);
  return (
    <table className="pf-table">
      <thead>
        <tr>
          <th>FUNCTION</th>
          <th className="num">A</th>
          <th className="num">B</th>
          <th className="num">DIFFERENCE</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => {
          const d = r.b - r.a;
          return (
            <tr key={`${r.name}@${r.file}`}>
              <td className="mono pf-ellipsis" title={r.file}>
                {frameLabel(r.name, r.file)}
              </td>
              <td className="num">{fmtValue(r.a, unit)}</td>
              <td className="num">{fmtValue(r.b, unit)}</td>
              <td className={`num ${d > 0 ? 'pf-more' : d < 0 ? 'pf-less' : ''}`}>
                {d > 0 ? '+' : d < 0 ? '−' : ''}
                {fmtValue(Math.abs(d), unit)}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

/** Largest increase and decrease of a frame from A to B, for the legend. */
function extremes(a: Map<string, number>, b: Map<string, number>): [number, number] {
  let less = 0;
  let more = 0;
  for (const [k, v] of b) {
    const o = a.get(k);
    if (o == null || k === 'all') continue;
    less = Math.min(less, v - o);
    more = Math.max(more, v - o);
  }
  return [less, more];
}

/** APM → Profiles → Comparison: the same metric over two time ranges or queries, side by side. */
export function ProfileComparison() {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const initial = params.get('q') ?? '';
  const [now] = useState(() => Date.now());
  const [a, setA] = useState<Side>({ durationMs: 3600e3, to: now - 3600e3, query: initial });
  const [b, setB] = useState<Side>({ durationMs: 3600e3, to: now, query: initial });
  const [sampleType, setSampleType] = useState('');
  const [endpoint, setEndpoint] = useState<string | null>(null);
  const [as, setAs] = useState<'flame' | 'table'>('flame');
  const [filter, setFilter] = useState('');
  const [expandAll, setExpandAll] = useState(false);
  const [groupBy, setGroupBy] = useState<GroupBy>('function');
  const ra = rangeOf(a);
  const rb = rangeOf(b);
  const span = { from: Math.min(ra.from, rb.from), to: Math.max(ra.to, rb.to) };

  const overview = useQuery({ queryKey: ['profiles', 'overview', span, b.query, sampleType], queryFn: () => profileOverview({ range: span, query: b.query, sampleType }) });
  const fa = useQuery({ queryKey: ['profiles', 'flame', ra, a.query, sampleType, endpoint], queryFn: () => flameGraph({ range: ra, query: a.query, sampleType, endpoint }), enabled: !!a.query });
  const fb = useQuery({ queryKey: ['profiles', 'flame', rb, b.query, sampleType, endpoint], queryFn: () => flameGraph({ range: rb, query: b.query, sampleType, endpoint }), enabled: !!b.query });
  const ta = useMemo(() => (fa.data ? viewTree(fa.data.root, expandAll, groupBy) : null), [fa.data, expandAll, groupBy]);
  const tb = useMemo(() => (fb.data ? viewTree(fb.data.root, expandAll, groupBy) : null), [fb.data, expandAll, groupBy]);
  const va = useMemo(() => (ta && fa.data ? valuesByKey(ta, fa.data.scale) : new Map<string, number>()), [ta, fa.data]);
  const vb = useMemo(() => (tb && fb.data ? valuesByKey(tb, fb.data.scale) : new Map<string, number>()), [tb, fb.data]);

  const marks = useMemo<EChartsOption['series']>(
    () => [
      {
        type: 'line',
        data: [],
        markArea: {
          silent: true,
          label: { position: 'insideTopLeft', color: '#fff', fontWeight: 'bold' },
          data: [
            [{ name: 'A', xAxis: ra.from, itemStyle: { color: token('--group-purple'), opacity: 0.12 } }, { xAxis: ra.to }],
            [{ name: 'B', xAxis: rb.from, itemStyle: { color: token('--group-blue'), opacity: 0.12 } }, { xAxis: rb.to }],
          ],
        },
      },
    ],
    [ra.from, ra.to, rb.from, rb.to],
  );

  const metric = fb.data?.metric ?? fa.data?.metric ?? overview.data?.metric ?? null;
  const types = fb.data?.cards ?? [];
  const [less, more] = extremes(va, vb);
  const open = (s: Side) => navigate(`/apm/profiles?q=${encodeURIComponent(s.query)}`);

  const pane = (name: 'A' | 'B', q: typeof fa, tree: ViewNode | null, other: Map<string, number>) => (
    <div className={`pf-pane pf-pane-${name}`}>
      <div className="pf-pane-head">
        <span className={`pf-pane-badge pf-side-${name}`}>{name}</span>
        {name === 'A' ? (
          <span className="pf-legend">
            <i className="pf-sw pf-sw-a" /> Only in A
          </span>
        ) : (
          <span className="pf-legend">
            <b>Less</b> {metric?.label ?? ''} in B <span className="pf-less-chip">−{fmtValue(Math.abs(less), metric?.unit ?? '')}</span>
            <span className="pf-more-chip">+{fmtValue(more, metric?.unit ?? '')}</span> <b>More</b> {metric?.label ?? ''} in B <i className="pf-sw pf-sw-b" /> Only in B
          </span>
        )}
      </div>
      {q.isPending ? (
        <Loading label="Merging profiles…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !q.data.metric || q.data.profiles === 0 ? (
        <Empty>No profiles for {name} in its time range.</Empty>
      ) : (
        tree && <FlameGraphView tree={tree} scale={q.data.scale} unit={q.data.metric.unit} title={averageLabel(q.data.metric)} filter={filter} diff={{ other, side: name }} />
      )}
    </div>
  );

  return (
    <div className="pf-compare-page">
      <div className="pf-sides">
        <SidePicker name="A" side={a} onChange={setA} onOpen={() => open(a)} />
        <button
          className="btn pf-swap"
          title="Swap A and B"
          onClick={() => {
            setA(b);
            setB(a);
          }}
        >
          <ArrowLeftRight size={16} />
        </button>
        <SidePicker name="B" side={b} onChange={setB} onOpen={() => open(b)} />
      </div>

      <div className="pf-chart">
        {overview.data?.metric && (
          <label className="pf-metric-select">
            <span>Metric</span>
            <b>{chartLabel(overview.data.metric)}</b>
          </label>
        )}
        {overview.data ? <MetricChart data={overview.data} marks={marks} /> : <div style={{ height: 150 }} />}
      </div>

      {!a.query || !b.query ? (
        <Empty>Filter both sides (for example service:storefront-api) to compare their profiles.</Empty>
      ) : (
        <>
          <div className="pf-compare-controls">
            <div className="pf-chipset">
              <span className="pf-chipset-label">Compare by</span>
              <select value={metric?.type ?? ''} onChange={(e) => setSampleType(e.target.value)}>
                {types.map((c) => (
                  <option key={c.metric.type} value={c.metric.type}>
                    {c.metric.label}
                  </option>
                ))}
              </select>
            </div>
            <div className="pf-chipset">
              <span className="pf-chipset-label">per</span>
              <select value="endpoints" disabled>
                <option value="endpoints">Endpoints</option>
              </select>
            </div>
            <div className="pf-chipset">
              <span className="pf-chipset-label">for</span>
              <select value={endpoint ?? '*'} onChange={(e) => setEndpoint(e.target.value === '*' ? null : e.target.value)}>
                <option value="*">All endpoints</option>
                {(fb.data?.endpoints ?? []).map((e) => (
                  <option key={e.name} value={e.name}>
                    {e.name || 'Other activity'}
                  </option>
                ))}
              </select>
            </div>
            <div className="btn-group">
              <span className="btn btn-label">as</span>
              <button className={`btn${as === 'flame' ? ' btn-active' : ''}`} onClick={() => setAs('flame')}>
                Flame Graph
              </button>
              <button className={`btn${as === 'table' ? ' btn-active' : ''}`} onClick={() => setAs('table')}>
                Table
              </button>
            </div>
          </div>
          {as === 'table' ? (
            fa.data && fb.data ? (
              <DiffTable a={fa.data} b={fb.data} />
            ) : (
              <Loading label="Merging profiles…" />
            )
          ) : (
            <>
              <div className="pf-flame-toolbar">
                <label className="pf-flame-filter">
                  <ListFilter size={15} />
                  <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter flame graph" />
                </label>
                <span className="pf-flame-tools">
                  <button className="pf-link-btn" onClick={() => setExpandAll((x) => !x)}>
                    <ChevronsUpDown size={15} /> {expandAll ? 'Collapse' : 'Expand All'}
                  </button>
                  <label className="pf-field pf-field-sm">
                    <span>Group by</span>
                    <select value={groupBy} onChange={(e) => setGroupBy(e.target.value as GroupBy)}>
                      <option value="function">Function</option>
                      <option value="package">Package</option>
                    </select>
                  </label>
                  <label className="pf-field pf-field-sm">
                    <span>Compare values</span>
                    <select value="absolute" disabled>
                      <option value="absolute">Absolute</option>
                    </select>
                  </label>
                </span>
              </div>
              <div className="pf-panes">
                {pane('A', fa, ta, vb)}
                {pane('B', fb, tb, va)}
              </div>
            </>
          )}
        </>
      )}
    </div>
  );
}
