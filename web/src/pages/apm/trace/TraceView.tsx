import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown, ChevronRight, Clock, Copy, ExternalLink, Link2, Maximize2, Minimize2, MinusSquare, PlusSquare, Search, X } from 'lucide-react';
import type { TraceDetail, TraceSpan } from '../../../api/apm/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { useSpanPercentile, useTrace } from '../queries';
import { fmtLatency } from '../visuals';
import { HttpStatus, ServiceGlyph, serviceColor } from '../explorer/SpanBits';
import { SpanPanel } from './SpanPanel';
import '../explorer/explorer.css';

type Tab = 'flame' | 'waterfall' | 'list' | 'map';
type ColorBy = 'service' | 'host' | 'status';

export interface Node {
  span: TraceSpan;
  depth: number;
  children: Node[];
  /** All descendants. */
  size: number;
}

export function httpCode(s: TraceSpan): string {
  return s.attributes['http.response.status_code'] || s.attributes['http.status_code'] || '';
}

export function httpMethod(s: TraceSpan): string {
  return s.attributes['http.request.method'] || s.attributes['http.method'] || '';
}

export function isError(s: TraceSpan): boolean {
  return s.status === 'Error';
}

function buildTree(spans: TraceSpan[]): Node[] {
  const ids = new Set(spans.map((s) => s.spanId));
  const nodes = new Map<string, Node>(spans.map((s) => [s.spanId, { span: s, depth: 0, children: [], size: 0 }]));
  const roots: Node[] = [];
  for (const s of spans) {
    const n = nodes.get(s.spanId)!;
    if (s.parentSpanId && ids.has(s.parentSpanId)) nodes.get(s.parentSpanId)!.children.push(n);
    else roots.push(n);
  }
  const visit = (n: Node, depth: number): number => {
    n.depth = depth;
    n.children.sort((a, b) => a.span.startUs - b.span.startUs);
    n.size = n.children.reduce((sum, c) => sum + 1 + visit(c, depth + 1), 0);
    return n.size;
  };
  roots.sort((a, b) => a.span.startUs - b.span.startUs).forEach((r) => visit(r, 0));
  return roots;
}

/** Side panel for one trace, opened from the explorer (`?trace=…&span=…`). */
export function TraceView({ traceId, focusSpanId, onClose, onSelectSpan }: { traceId: string; focusSpanId: string | null; onClose: () => void; onSelectSpan: (id: string) => void }) {
  const q = useTrace(traceId);
  const [full, setFull] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return createPortal(
    <div className="tv-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className={`tv${full ? ' full' : ''}`} role="dialog" aria-label={`Trace ${traceId}`}>
        {q.isPending ? (
          <Loading label="Loading trace…" />
        ) : q.isError ? (
          <>
            <button className="icon-plain tv-close-alone" onClick={onClose} title="Close (Esc)">
              <X size={20} />
            </button>
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          </>
        ) : (
          <TraceBody trace={q.data} focusSpanId={focusSpanId} onSelectSpan={onSelectSpan} onClose={onClose} full={full} setFull={setFull} />
        )}
      </aside>
    </div>,
    document.body,
  );
}

function TraceBody({
  trace,
  focusSpanId,
  onSelectSpan,
  onClose,
  full,
  setFull,
}: {
  trace: TraceDetail;
  focusSpanId: string | null;
  onSelectSpan: (id: string) => void;
  onClose: () => void;
  full: boolean;
  setFull: (f: boolean) => void;
}) {
  const roots = useMemo(() => buildTree(trace.spans), [trace.spans]);
  const root = roots[0]?.span;
  const [selected, setSelected] = useState<string>(focusSpanId && trace.spans.some((s) => s.spanId === focusSpanId) ? focusSpanId : (root?.spanId ?? ''));
  const [tab, setTab] = useState<Tab>('waterfall');
  const [filter, setFilter] = useState('');
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [colorBy, setColorBy] = useState<ColorBy>('service');
  const [copied, setCopied] = useState<string | null>(null);
  const span = trace.spans.find((s) => s.spanId === selected) ?? root;
  const errorCount = trace.spans.filter(isError).length;
  const pct = useSpanPercentile(root?.service ?? '', root?.name ?? '', (root?.durationUs ?? 0) / 1000, trace.start);
  const select = (id: string) => {
    setSelected(id);
    onSelectSpan(id);
  };
  const copy = (what: string, text: string) => {
    void navigator.clipboard?.writeText(text);
    setCopied(what);
    setTimeout(() => setCopied(null), 1200);
  };
  const link = `${window.location.origin}/apm/traces?trace=${trace.traceId}${selected ? `&span=${selected}` : ''}`;
  const code = root ? httpCode(root) : '';
  const method = root ? httpMethod(root) : '';
  const color = (s: TraceSpan) => (colorBy === 'status' ? (isError(s) ? '#e5383b' : '#3ec46d') : serviceColor(colorBy === 'host' ? s.resource['host.name'] || 'unknown' : s.service));
  // Spans that match the filter (null = no filter).
  const matches = useMemo(() => {
    const f = filter.trim().toLowerCase();
    if (!f && !errorsOnly) return null;
    return new Set(
      trace.spans
        .filter((s) => (!errorsOnly || isError(s)) && (!f || [s.service, s.name, ...Object.values(s.attributes), ...Object.values(s.resource)].some((v) => v.toLowerCase().includes(f))))
        .map((s) => s.spanId),
    );
  }, [trace.spans, filter, errorsOnly]);

  return (
    <div className="tv-body">
      <header className={`tv-head${errorCount ? ' err' : ''}`}>
        <div className="tv-title">
          {root && <ServiceGlyph service={root.service} kind={root.kind.toLowerCase()} size={24} />}
          <b>{root?.service}</b>
          <ChevronRight size={16} className="muted" />
          <span className="tv-resource" title={root?.name}>
            {root?.name}
          </span>
          <button className="icon-plain" title="Copy resource" onClick={() => root && copy('res', root.name)}>
            {copied === 'res' ? <Check size={14} /> : <Copy size={14} />}
          </button>
          <ChevronRight size={16} className="muted" />
          <span className="tv-tid">
            trace_id <code>{trace.traceId}</code>
          </span>
          <button className="icon-plain" title="Copy trace ID" onClick={() => copy('tid', trace.traceId)}>
            {copied === 'tid' ? <Check size={14} /> : <Copy size={14} />}
          </button>
          <span className="tv-actions">
            <button className="icon-plain" title="Copy link to this trace" onClick={() => copy('link', link)}>
              {copied === 'link' ? <Check size={18} /> : <Link2 size={18} />}
            </button>
            <button className="icon-plain" title={full ? 'Exit full width' : 'Full width'} onClick={() => setFull(!full)}>
              {full ? <Minimize2 size={18} /> : <Maximize2 size={18} />}
            </button>
            <button className="icon-plain" title="Open in a new tab" onClick={() => window.open(link, '_blank', 'noopener')}>
              <ExternalLink size={18} />
            </button>
            <button className="icon-plain" title="Close (Esc)" onClick={onClose}>
              <X size={20} />
            </button>
          </span>
        </div>
        <div className="tv-chips">
          {pct.data && pct.data.peers > 1 && (
            <span className="tv-pct" title={`Slower than ${pct.data.percentile}% of ${pct.data.peers.toLocaleString()} ${root?.name} spans in the previous hour`}>
              p{pct.data.percentile}
            </span>
          )}
          <span className="tv-dur">
            <Clock size={15} /> {fmtLatency(trace.durationMs)}
          </span>
          {method && <span className="tv-method">{method}</span>}
          {code && <HttpStatus code={code} long />}
          {!code && errorCount > 0 && <span className="sx-code-badge err">ERROR</span>}
          <span className="muted">
            {new Date(trace.start).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })} ({ago(trace.start)})
          </span>
        </div>
      </header>
      <nav className="tv-tabs">
        <span className="muted">Trace:</span>
        {(
          [
            ['flame', 'Flame Graph'],
            ['waterfall', 'Waterfall'],
            ['list', 'Span List'],
            ['map', 'Map'],
          ] as [Tab, string][]
        ).map(([id, label]) => (
          <button key={id} className={tab === id ? 'active' : ''} onClick={() => setTab(id)}>
            {label}
            {id === 'list' && <span className="tv-count">{trace.spans.length}</span>}
          </button>
        ))}
      </nav>
      <div className="tv-tools">
        <label className="tv-filter">
          <Search size={15} />
          <input placeholder="Filter spans by any attribute" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </label>
        <label className="tv-errors">
          <input type="checkbox" className="checkbox" checked={errorsOnly} onChange={() => setErrorsOnly((v) => !v)} /> Error {errorCount > 0 && <span className="tv-err-count">{errorCount}</span>}
        </label>
        <label className="tv-colorby">
          Color by
          <select value={colorBy} onChange={(e) => setColorBy(e.target.value as ColorBy)}>
            <option value="service">Service</option>
            <option value="host">Host</option>
            <option value="status">Status</option>
          </select>
        </label>
      </div>
      <div className="tv-viz">
        {tab === 'waterfall' && <Waterfall roots={roots} total={trace.durationMs * 1000} selected={span?.spanId} onSelect={select} matches={matches} color={color} />}
        {tab === 'flame' && <Flame roots={roots} total={trace.durationMs * 1000} selected={span?.spanId} onSelect={select} matches={matches} color={color} />}
        {tab === 'list' && <SpanListTab spans={trace.spans} total={trace.durationMs * 1000} onSelect={select} matches={matches} />}
        {tab === 'map' && <ServiceMap spans={trace.spans} />}
      </div>
      {span && <SpanPanel span={span} trace={trace} />}
    </div>
  );
}

function ago(ms: number): string {
  const s = (Date.now() - ms) / 1000;
  if (s < 60) return `${Math.round(s)}s ago`;
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

function ticks(totalUs: number): number[] {
  const ms = totalUs / 1000;
  const steps = [0.1, 0.25, 0.5, 1, 2, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10_000, 30_000, 60_000];
  const step = steps.find((s) => ms / s <= 8) ?? 120_000;
  const out: number[] = [];
  for (let t = 0; t <= ms; t += step) out.push(t);
  return out;
}

interface VizProps {
  roots: Node[];
  total: number;
  selected?: string;
  onSelect: (id: string) => void;
  matches: Set<string> | null;
  color: (s: TraceSpan) => string;
}

function Waterfall({ roots, total, selected, onSelect, matches, color }: VizProps) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const rows: Node[] = [];
  const walk = (n: Node) => {
    rows.push(n);
    if (!collapsed.has(n.span.spanId)) n.children.forEach(walk);
  };
  roots.forEach(walk);
  const all = useMemo(() => {
    const ids: string[] = [];
    const w = (n: Node) => {
      if (n.children.length) ids.push(n.span.spanId);
      n.children.forEach(w);
    };
    roots.forEach(w);
    return ids;
  }, [roots]);
  const t = Math.max(total, 1);
  return (
    <div className="wf2">
      <div className="wf2-head">
        <span className="wf2-tools">
          <button className="icon-plain" title="Expand all" onClick={() => setCollapsed(new Set())}>
            <PlusSquare size={16} />
          </button>
          <button className="icon-plain" title="Collapse all" onClick={() => setCollapsed(new Set(all))}>
            <MinusSquare size={16} />
          </button>
        </span>
        <span className="wf2-axis">
          {ticks(total).map((ms) => (
            <span key={ms} style={{ left: `${((ms * 1000) / t) * 100}%` }}>
              {fmtLatency(ms)}
            </span>
          ))}
        </span>
      </div>
      {rows.map((n) => {
        const s = n.span;
        const dim = matches && !matches.has(s.spanId);
        const left = (s.startUs / t) * 100;
        const width = Math.max((s.durationUs / t) * 100, 0.25);
        const code = httpCode(s);
        return (
          <div key={s.spanId} className={`wf2-row${s.spanId === selected ? ' selected' : ''}${dim ? ' dim' : ''}`} onClick={() => onSelect(s.spanId)}>
            <span className="wf2-label" style={{ paddingLeft: n.depth * 16 }}>
              {n.children.length > 0 ? (
                <button
                  className="icon-plain wf2-toggle"
                  onClick={(e) => {
                    e.stopPropagation();
                    const c = new Set(collapsed);
                    if (c.has(s.spanId)) c.delete(s.spanId);
                    else c.add(s.spanId);
                    setCollapsed(c);
                  }}
                >
                  {collapsed.has(s.spanId) ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
                </button>
              ) : (
                <span className="wf2-toggle" />
              )}
              {n.size > 0 && <span className="wf2-size">{n.size}</span>}
              <ServiceGlyph service={s.service} kind={s.kind.toLowerCase()} dbSystem={s.attributes['db.system']} size={16} />
              <b>{s.service}</b>
              <span className="wf2-name" title={s.name}>
                {s.name}
              </span>
              {isError(s) && <span className="wf2-errdot" title={s.statusMessage || 'Error'} />}
            </span>
            <span className={`wf2-code ${code ? (code.startsWith('5') ? 'err' : code.startsWith('4') ? 'warn' : 'ok') : ''}`}>{code}</span>
            <span className="wf2-track">
              <span className={`wf2-bar${isError(s) ? ' err' : ''}`} style={{ left: `${left}%`, width: `${width}%`, background: color(s) }} />
            </span>
            <span className="wf2-dur">{fmtLatency(s.durationUs / 1000)}</span>
          </div>
        );
      })}
    </div>
  );
}

function Flame({ roots, total, selected, onSelect, matches, color }: VizProps) {
  const rows: Node[][] = [];
  const walk = (n: Node) => {
    (rows[n.depth] ??= []).push(n);
    n.children.forEach(walk);
  };
  roots.forEach(walk);
  const t = Math.max(total, 1);
  return (
    <div className="fg">
      <div className="wf2-axis fg-axis">
        {ticks(total).map((ms) => (
          <span key={ms} style={{ left: `${((ms * 1000) / t) * 100}%` }}>
            {fmtLatency(ms)}
          </span>
        ))}
      </div>
      {rows.map((row, d) => (
        <div key={d} className="fg-row">
          {row.map((n) => {
            const s = n.span;
            return (
              <button
                key={s.spanId}
                className={`fg-bar${s.spanId === selected ? ' selected' : ''}${matches && !matches.has(s.spanId) ? ' dim' : ''}${isError(s) ? ' err' : ''}`}
                style={{ left: `${(s.startUs / t) * 100}%`, width: `${Math.max((s.durationUs / t) * 100, 0.2)}%`, background: color(s) }}
                title={`${s.service} ${s.name} · ${fmtLatency(s.durationUs / 1000)}`}
                onClick={() => onSelect(s.spanId)}
              >
                <span>
                  {s.service} {s.name}
                </span>
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}

function SpanListTab({ spans, total, onSelect, matches }: { spans: TraceSpan[]; total: number; onSelect: (id: string) => void; matches: Set<string> | null }) {
  // Exclusive ("self") time: a span's duration minus the time covered by its children.
  const groups = useMemo(() => {
    const kids = new Map<string, TraceSpan[]>();
    for (const s of spans) kids.set(s.parentSpanId, [...(kids.get(s.parentSpanId) ?? []), s]);
    const self = (s: TraceSpan) => {
      const iv = (kids.get(s.spanId) ?? []).map((c) => [Math.max(c.startUs, s.startUs), Math.min(c.startUs + c.durationUs, s.startUs + s.durationUs)] as [number, number]).sort((a, b) => a[0] - b[0]);
      let covered = 0;
      let end = -Infinity;
      for (const [a, b] of iv) {
        if (b <= end) continue;
        covered += b - Math.max(a, end);
        end = b;
      }
      return Math.max(0, s.durationUs - covered);
    };
    const m = new Map<string, { service: string; name: string; count: number; errors: number; total: number; self: number; first: string }>();
    for (const s of spans) {
      if (matches && !matches.has(s.spanId)) continue;
      const k = `${s.service}|${s.name}`;
      const g = m.get(k) ?? { service: s.service, name: s.name, count: 0, errors: 0, total: 0, self: 0, first: s.spanId };
      g.count++;
      g.errors += isError(s) ? 1 : 0;
      g.total += s.durationUs;
      g.self += self(s);
      m.set(k, g);
    }
    return [...m.values()].sort((a, b) => b.self - a.self);
  }, [spans, matches]);
  return (
    <table className="sx-table tv-list">
      <thead>
        <tr>
          <th>SERVICE</th>
          <th>RESOURCE</th>
          <th className="num">SPANS</th>
          <th className="num">ERRORS</th>
          <th className="num">AVG DURATION</th>
          <th className="num">EXEC TIME</th>
          <th className="num">% EXEC TIME</th>
        </tr>
      </thead>
      <tbody>
        {groups.map((g) => (
          <tr key={`${g.service}|${g.name}`} onClick={() => onSelect(g.first)}>
            <td>
              <span className="sx-svc">
                <ServiceGlyph service={g.service} size={16} /> {g.service}
              </span>
            </td>
            <td className="sx-resource">{g.name}</td>
            <td className="num">{g.count}</td>
            <td className="num">{g.errors ? <span className="sx-err">{g.errors}</span> : 0}</td>
            <td className="num">{fmtLatency(g.total / g.count / 1000)}</td>
            <td className="num">{fmtLatency(g.self / 1000)}</td>
            <td className="num">{((g.self / Math.max(total, 1)) * 100).toFixed(1)}%</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Services in this trace and the calls between them (parent span service → child span service). */
function ServiceMap({ spans }: { spans: TraceSpan[] }) {
  const { nodes, edges, cols } = useMemo(() => {
    const byId = new Map(spans.map((s) => [s.spanId, s]));
    const depth = new Map<string, number>();
    const edges = new Map<string, { from: string; to: string; calls: number; errors: number }>();
    const nodeStats = new Map<string, { spans: number; errors: number }>();
    const levelOf = (s: TraceSpan): number => {
      let d = 0;
      let p = byId.get(s.parentSpanId);
      let last = s.service;
      while (p) {
        if (p.service !== last) d++;
        last = p.service;
        p = byId.get(p.parentSpanId);
      }
      return d;
    };
    for (const s of spans) {
      const st = nodeStats.get(s.service) ?? { spans: 0, errors: 0 };
      st.spans++;
      st.errors += isError(s) ? 1 : 0;
      nodeStats.set(s.service, st);
      const d = levelOf(s);
      depth.set(s.service, Math.min(depth.get(s.service) ?? d, d));
      const p = byId.get(s.parentSpanId);
      if (p && p.service !== s.service) {
        const k = `${p.service}→${s.service}`;
        const e = edges.get(k) ?? { from: p.service, to: s.service, calls: 0, errors: 0 };
        e.calls++;
        e.errors += isError(s) ? 1 : 0;
        edges.set(k, e);
      }
    }
    const cols: string[][] = [];
    for (const [svc, d] of depth) (cols[d] ??= []).push(svc);
    cols.forEach((c) => c.sort());
    return { nodes: nodeStats, edges: [...edges.values()], cols };
  }, [spans]);
  const W = 220;
  const H = 64;
  const GX = 110;
  const GY = 26;
  const pos = new Map<string, { x: number; y: number }>();
  cols.forEach((c, i) => c?.forEach((svc, j) => pos.set(svc, { x: 20 + i * (W + GX), y: 20 + j * (H + GY) })));
  const width = 40 + cols.length * (W + GX);
  const height = 40 + Math.max(...cols.map((c) => c?.length ?? 0), 1) * (H + GY);
  return (
    <div className="tv-map">
      <svg width={width} height={height}>
        <defs>
          <marker id="tv-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
            <path d="M 0 0 L 10 5 L 0 10 z" fill="#8b93a3" />
          </marker>
        </defs>
        {edges.map((e) => {
          const a = pos.get(e.from)!;
          const b = pos.get(e.to)!;
          const x1 = a.x + W;
          const y1 = a.y + H / 2;
          const x2 = b.x;
          const y2 = b.y + H / 2;
          const mx = (x1 + x2) / 2;
          return (
            <g key={`${e.from}→${e.to}`}>
              <path d={`M ${x1} ${y1} C ${mx} ${y1}, ${mx} ${y2}, ${x2 - 2} ${y2}`} fill="none" stroke={e.errors ? '#e5383b' : '#8b93a3'} strokeWidth={1.5} markerEnd="url(#tv-arrow)" />
              <text x={mx} y={(y1 + y2) / 2 - 6} textAnchor="middle" className="tv-map-edge">
                {e.calls} call{e.calls === 1 ? '' : 's'}
                {e.errors ? ` · ${e.errors} err` : ''}
              </text>
            </g>
          );
        })}
        {[...pos].map(([svc, p]) => {
          const st = nodes.get(svc)!;
          return (
            <g key={svc} transform={`translate(${p.x},${p.y})`}>
              <rect width={W} height={H} rx={8} className={`tv-map-node${st.errors ? ' err' : ''}`} />
              <rect width={6} height={H} rx={3} fill={serviceColor(svc)} />
              <text x={16} y={26} className="tv-map-name">
                {svc.length > 26 ? `${svc.slice(0, 25)}…` : svc}
              </text>
              <text x={16} y={46} className="tv-map-sub">
                {st.spans} span{st.spans === 1 ? '' : 's'}
                {st.errors ? ` · ${st.errors} error${st.errors === 1 ? '' : 's'}` : ''}
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}
