import { useMemo } from 'react';
import type { StreamEdge, StreamNode } from '../../api/dataStreams/client';
import { fmtCount, fmtMs, fmtRate } from './format';
import { edgeMid, edgePath, layoutGraph } from './mapLayout';

export type EdgeLabel = 'throughput' | 'latency';

const SERVICE_HALF = 84;
const TOPIC_HALF = 72;
const half = (n: StreamNode) => (n.kind === 'topic' ? TOPIC_HALF : SERVICE_HALF);
const short = (s: string, max: number) => (s.length > max ? `${s.slice(0, max - 1)}…` : s);

function label(e: StreamEdge, mode: EdgeLabel): string {
  if (mode === 'latency') return e.kind === 'consume' ? fmtMs(e.latencyP50Ms) : fmtMs(e.spanP50Ms);
  return fmtRate(e.throughput);
}

/** A service ⇄ topic graph: services as boxes, topics as cylinders, edges with throughput or latency. */
export function StreamGraph({
  nodes,
  edges,
  selected,
  onSelect,
  edgeLabel = 'throughput',
  dimmed,
  fit = false,
}: {
  nodes: StreamNode[];
  edges: StreamEdge[];
  selected?: string | null;
  onSelect?: (id: string | null) => void;
  edgeLabel?: EdgeLabel;
  /** Nodes to fade (filtered out). */
  dimmed?: Set<string>;
  /** Scale to the container's width instead of scrolling. */
  fit?: boolean;
}) {
  const layout = useMemo(
    () =>
      layoutGraph(
        nodes.map((n) => n.id),
        edges,
      ),
    [nodes, edges],
  );
  const byId = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);
  const maxRate = Math.max(1e-9, ...edges.map((e) => e.throughput));
  const near = selected ? new Set(edges.filter((e) => e.source === selected || e.target === selected).flatMap((e) => [e.source, e.target])) : null;
  const faded = (id: string) => (dimmed?.has(id) ?? false) || (near != null && !near.has(id));
  return (
    <svg
      className="dsm-graph"
      width={fit ? '100%' : layout.width}
      height={fit ? undefined : layout.height}
      viewBox={`0 0 ${layout.width} ${layout.height}`}
      role="img"
      aria-label="Data streams map"
      onClick={() => onSelect?.(null)}
    >
      <defs>
        <marker id="dsm-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="9" markerHeight="9" orient="auto-start-reverse">
          <path d="M0,0 L10,5 L0,10 z" className="dsm-arrow" />
        </marker>
        <marker id="dsm-arrow-lag" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="9" markerHeight="9" orient="auto-start-reverse">
          <path d="M0,0 L10,5 L0,10 z" className="dsm-arrow-lag" />
        </marker>
      </defs>
      {edges.map((e) => {
        const a = layout.nodes.get(e.source);
        const b = layout.nodes.get(e.target);
        const na = byId.get(e.source);
        const nb = byId.get(e.target);
        if (!a || !b || !na || !nb) return null;
        // edgePath takes one half width: shift both ends so each meets its own node's side.
        const from = { x: a.x + half(na) - SERVICE_HALF, y: a.y };
        const to = { x: b.x - half(nb) + SERVICE_HALF, y: b.y };
        const mid = edgeMid(a, b);
        const text = label(e, edgeLabel);
        const off = faded(e.source) || faded(e.target);
        return (
          <g key={`${e.source}>${e.target}`} className={`dsm-edge${e.lagHigh ? ' lag' : ''}${off ? ' faded' : ''}`}>
            <title>
              {`${na.name} → ${nb.name}\n${fmtRate(e.throughput)}${e.kind === 'consume' && e.latencyP50Ms != null ? `\nPublish → consume p50 ${fmtMs(e.latencyP50Ms)}, p99 ${fmtMs(e.latencyP99Ms)}` : ''}${e.lag != null ? `\nConsumer lag ${fmtCount(e.lag)} messages` : ''}`}
            </title>
            <path d={edgePath(from, to, SERVICE_HALF)} strokeWidth={1.4 + 3.6 * Math.sqrt(e.throughput / maxRate)} markerEnd={`url(#${e.lagHigh ? 'dsm-arrow-lag' : 'dsm-arrow'})`} />
            <g transform={`translate(${mid.x},${mid.y})`}>
              <rect x={-(text.length * 3.3 + 8)} y={-10} width={text.length * 6.6 + 16} height={20} rx={10} className="dsm-edge-label" />
              <text textAnchor="middle" dy="4" className="dsm-edge-text">
                {text}
              </text>
              {e.lagHigh && (
                <g transform="translate(0,22)">
                  <rect x={-38} y={-9} width={76} height={18} rx={9} className="dsm-lag-pill" />
                  <text textAnchor="middle" dy="4" className="dsm-lag-text">
                    Lag {fmtCount(e.lag)}
                  </text>
                </g>
              )}
            </g>
          </g>
        );
      })}
      {nodes.map((n) => {
        const p = layout.nodes.get(n.id);
        if (!p) return null;
        const cls = `dsm-node ${n.kind}${n.id === selected ? ' selected' : ''}${n.lagHigh ? ' lag' : ''}${faded(n.id) ? ' faded' : ''}`;
        const sub = n.kind === 'topic' ? `${fmtRate(n.produced)} in` : n.consumed > 0 ? `${fmtRate(n.consumed)} consumed` : `${fmtRate(n.produced)} published`;
        return (
          <g
            key={n.id}
            className={cls}
            transform={`translate(${p.x},${p.y})`}
            onClick={(ev) => {
              ev.stopPropagation();
              onSelect?.(n.id === selected ? null : n.id);
            }}
            role={onSelect ? 'button' : undefined}
            tabIndex={onSelect ? 0 : undefined}
            onKeyDown={(ev) => {
              if (ev.key === 'Enter') onSelect?.(n.id);
            }}
          >
            <title>{n.name}</title>
            {n.kind === 'topic' ? (
              <>
                <path className="dsm-shape" d={`M${-TOPIC_HALF},-17 v34 a${TOPIC_HALF},7 0 0,0 ${TOPIC_HALF * 2},0 v-34`} />
                <ellipse className="dsm-shape dsm-cap" cx={0} cy={-17} rx={TOPIC_HALF} ry={7} />
                <text className="dsm-node-name" textAnchor="middle" y={4}>
                  {short(n.name, 22)}
                </text>
                <text className="dsm-node-sub" textAnchor="middle" y={19}>
                  {sub}
                </text>
              </>
            ) : (
              <>
                <rect className="dsm-shape" x={-SERVICE_HALF} y={-23} width={SERVICE_HALF * 2} height={46} rx={8} />
                <circle className="dsm-service-dot" cx={-SERVICE_HALF + 18} cy={0} r={9} />
                <text className="dsm-service-initial" x={-SERVICE_HALF + 18} y={4} textAnchor="middle">
                  {n.name.slice(0, 1).toUpperCase()}
                </text>
                <text className="dsm-node-name" x={-SERVICE_HALF + 34} y={-3}>
                  {short(n.name, 19)}
                </text>
                <text className="dsm-node-sub" x={-SERVICE_HALF + 34} y={13}>
                  {sub}
                </text>
              </>
            )}
            {n.lagHigh && (
              <g transform={`translate(${half(n) - 6},${n.kind === 'topic' ? -20 : -20})`}>
                <circle r={8} className="dsm-badge" />
                <text textAnchor="middle" dy="4" className="dsm-badge-text">
                  !
                </text>
              </g>
            )}
          </g>
        );
      })}
    </svg>
  );
}
