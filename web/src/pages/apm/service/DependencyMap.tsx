import { useRef, useState, type ReactNode } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import type { Dependencies, Dependency } from '../../../api/apm/client';
import { BrandLogo } from '../../../components/brand/BrandLogo';
import { fmtLatency, fmtPct, fmtRate, serviceLogo } from '../visuals';
import { dependencyHealth, dependencyLinks, isInferred, trafficShare } from './logic';

const W = 900;
const ROW = 54;
const NODE_R = 13;
/** Logo size inside a node. */
const LOGO = 16;
const HEALTH_LABEL = { ok: 'OK', warn: 'WARN', critical: 'CRITICAL' } as const;

type Side = 'upstream' | 'downstream';

/** The node under the pointer, at its position in the map (fractions of its width and height). */
interface Hovered {
  d: Dependency;
  side: Side;
  left: number;
  top: number;
}

function edgeColor(d: Dependency): string {
  return d.errorRate >= 0.05 ? 'var(--status-error)' : d.errorRate >= 0.01 ? 'var(--status-warn)' : 'var(--border-strong)';
}

/** A service's logo centred on (x, y), on a white disc so it reads on any kind colour. */
function Logo({ name, language, technology, x, y, size }: { name: string; language?: string | null; technology?: string | null; x: number; y: number; size: number }) {
  const logo = serviceLogo(name, language, technology);
  if (!logo) return null;
  return (
    <g transform={`translate(${x - size / 2} ${y - size / 2})`} pointerEvents="none">
      <circle cx={size / 2} cy={size / 2} r={size / 2 + 2} className="dm-logo-bg" />
      <BrandLogo name={logo} size={size} />
    </g>
  );
}

/** A node's hover card: health, share of its side's traffic, errors, latency and throughput, and its links. */
function NodeCard({ service, h, side, onEnter, onLeave }: { service: string; h: Hovered; side: Dependency[]; onEnter: () => void; onLeave: () => void }) {
  const health = dependencyHealth(h.d.errorRate);
  const links = dependencyLinks(service, h.d, h.side);
  const row = (label: string, value: ReactNode) => (
    <div className="dm-card-row">
      <span className="muted">{label}</span>
      <span>{value}</span>
    </div>
  );
  return (
    <div
      className={`dm-card ${h.side}`}
      style={{ left: `${h.left * 100}%`, top: `${h.top * 100}%` }}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      role="dialog"
      aria-label={`${h.d.service} dependency`}
    >
      <div className="dm-card-title">
        {h.d.service}
        {isInferred(h.d.kind) && <span className="muted"> · inferred</span>}
      </div>
      {row('Health', <span className={`health-badge health-${health}`}>{HEALTH_LABEL[health]}</span>)}
      {row(h.side === 'upstream' ? '% of incoming traffic' : '% of outgoing traffic', fmtPct(trafficShare(h.d, side)))}
      {row('Error rate', fmtPct(h.d.errorRate))}
      {row('p95 latency', fmtLatency(h.d.p95Ms))}
      {row('Requests', fmtRate(h.d.reqPerSec))}
      <div className="dm-card-links">
        <Link to={links.service}>View Service Page</Link>
        <Link to={links.traces}>View Traces</Link>
      </div>
    </div>
  );
}

/** Upstream callers → this service → downstream dependencies. Edge width ∝ log(req/s), colour = error rate; inferred
 * services (databases, caches, queues, hosts) are dashed. */
export function DependencyMap({ service, deps }: { service: string; deps: Dependencies }) {
  const navigate = useNavigate();
  const [hovered, setHovered] = useState<Hovered | null>(null);
  const hide = useRef<number | undefined>(undefined);
  const rows = Math.max(deps.upstream.length, deps.downstream.length, 1);
  const H = rows * ROW + 24;
  const cx = W / 2;
  const cy = H / 2;
  const maxRate = Math.max(0.001, ...[...deps.upstream, ...deps.downstream].map((d) => d.reqPerSec));
  const width = (d: Dependency) => 1 + 4 * (Math.log1p(d.reqPerSec) / Math.log1p(maxRate));
  const ys = (n: number) => Array.from({ length: n }, (_, i) => cy + (i - (n - 1) / 2) * ROW);
  const up = ys(deps.upstream.length);
  const down = ys(deps.downstream.length);
  const keep = () => window.clearTimeout(hide.current);
  const leave = () => (hide.current = window.setTimeout(() => setHovered(null), 150));

  const node = (d: Dependency, x: number, y: number, side: Side) => {
    const anchor = side === 'upstream' ? 'end' : 'start';
    const tx = side === 'upstream' ? x - 20 : x + 20;
    return (
      <g
        key={`${side}:${d.service}`}
        className={`dm-node${isInferred(d.kind) ? ' inferred' : ''}`}
        onClick={() => navigate(`/apm/services/${encodeURIComponent(d.service)}`)}
        onMouseEnter={() => (keep(), setHovered({ d, side, left: x / W, top: y / H }))}
        onMouseLeave={leave}
      >
        <circle cx={x} cy={y} r={NODE_R} className={`dm-kind-${d.kind}`} />
        <Logo name={d.service} language={d.language} technology={d.technology} x={x} y={y} size={LOGO} />
        <text x={tx} y={y} textAnchor={anchor}>
          {d.service}
        </text>
        <text x={tx} y={y + 15} textAnchor={anchor} className="dm-stats">
          {fmtRate(d.reqPerSec)} · {fmtPct(d.errorRate)} err
        </text>
      </g>
    );
  };
  const edge = (d: Dependency, path: string, side: Side) => (
    <path key={`${side}e:${d.service}`} d={path} stroke={edgeColor(d)} strokeWidth={width(d)} strokeDasharray={isInferred(d.kind) ? '5 4' : undefined} fill="none" />
  );

  if (deps.upstream.length === 0 && deps.downstream.length === 0) return <div className="hp-empty">No dependencies observed in this time range</div>;

  return (
    <div className="dep-map">
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" style={{ maxHeight: 480 }} role="img" aria-label={`Dependency map for ${service}`}>
        {deps.upstream.map((d, i) => edge(d, `M ${200} ${up[i]} C ${cx - 120} ${up[i]}, ${cx - 120} ${cy}, ${cx - 26} ${cy}`, 'upstream'))}
        {deps.downstream.map((d, i) => edge(d, `M ${cx + 26} ${cy} C ${cx + 120} ${cy}, ${cx + 120} ${down[i]}, ${W - 200} ${down[i]}`, 'downstream'))}
        {deps.upstream.map((d, i) => node(d, 200, up[i], 'upstream'))}
        {deps.downstream.map((d, i) => node(d, W - 200, down[i], 'downstream'))}
        <g className="dm-center">
          <circle cx={cx} cy={cy} r={22} />
          <Logo name={service} language={deps.language} x={cx} y={cy} size={24} />
          <text x={cx} y={cy + 40} textAnchor="middle">
            {service}
          </text>
        </g>
      </svg>
      {hovered && <NodeCard service={service} h={hovered} side={hovered.side === 'upstream' ? deps.upstream : deps.downstream} onEnter={keep} onLeave={leave} />}
    </div>
  );
}
