import { useNavigate } from 'react-router-dom';
import type { Dependencies, Dependency } from '../../../api/apm/client';
import { BrandLogo } from '../../../components/brand/BrandLogo';
import { fmtPct, fmtRate, serviceLogo } from '../visuals';

const W = 900;
const ROW = 46;
const NODE_R = 13;
/** Logo size inside a node. */
const LOGO = 16;

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

/** Upstream callers → this service → downstream dependencies. Edge width ∝ log(req/s), colour = error rate. */
export function DependencyMap({ service, deps }: { service: string; deps: Dependencies }) {
  const navigate = useNavigate();
  const rows = Math.max(deps.upstream.length, deps.downstream.length, 1);
  const H = rows * ROW + 24;
  const cx = W / 2;
  const cy = H / 2;
  const maxRate = Math.max(0.001, ...[...deps.upstream, ...deps.downstream].map((d) => d.reqPerSec));
  const width = (d: Dependency) => 1 + 4 * (Math.log1p(d.reqPerSec) / Math.log1p(maxRate));
  const ys = (n: number) => Array.from({ length: n }, (_, i) => cy + (i - (n - 1) / 2) * ROW);
  const up = ys(deps.upstream.length);
  const down = ys(deps.downstream.length);
  const open = (s: string) => navigate(`/apm/services/${encodeURIComponent(s)}`);

  const node = (d: Dependency, x: number, y: number, anchor: 'start' | 'end') => (
    <g key={`${anchor}:${d.service}`} className="dm-node" onClick={() => open(d.service)}>
      <title>{`${d.service}\n${fmtRate(d.reqPerSec)} · ${fmtPct(d.errorRate)} errors`}</title>
      <circle cx={x} cy={y} r={NODE_R} className={`dm-kind-${d.kind}`} />
      <Logo name={d.service} language={d.language} technology={d.technology} x={x} y={y} size={LOGO} />
      <text x={anchor === 'end' ? x - 20 : x + 20} y={y + 4} textAnchor={anchor}>
        {d.service}
      </text>
    </g>
  );

  if (deps.upstream.length === 0 && deps.downstream.length === 0) return <div className="hp-empty">No dependencies observed in this time range</div>;

  return (
    <div className="dep-map">
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" style={{ maxHeight: 420 }} role="img" aria-label={`Dependency map for ${service}`}>
        {deps.upstream.map((d, i) => (
          <path key={`ue:${d.service}`} d={`M ${200} ${up[i]} C ${cx - 120} ${up[i]}, ${cx - 120} ${cy}, ${cx - 26} ${cy}`} stroke={edgeColor(d)} strokeWidth={width(d)} fill="none" />
        ))}
        {deps.downstream.map((d, i) => (
          <path key={`de:${d.service}`} d={`M ${cx + 26} ${cy} C ${cx + 120} ${cy}, ${cx + 120} ${down[i]}, ${W - 200} ${down[i]}`} stroke={edgeColor(d)} strokeWidth={width(d)} fill="none" />
        ))}
        {deps.upstream.map((d, i) => node(d, 200, up[i], 'end'))}
        {deps.downstream.map((d, i) => node(d, W - 200, down[i], 'start'))}
        <g className="dm-center">
          <circle cx={cx} cy={cy} r={22} />
          <Logo name={service} language={deps.language} x={cx} y={cy} size={24} />
          <text x={cx} y={cy + 40} textAnchor="middle">
            {service}
          </text>
        </g>
      </svg>
    </div>
  );
}
