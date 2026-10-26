import { memo } from 'react';

/** Deterministic low-poly backdrop: a jittered triangle mesh shaded along a deep teal → blue-teal diagonal. */
export const LowPoly = memo(function LowPoly() {
  const W = 1600;
  const H = 1000;
  const COLS = 17;
  const ROWS = 11;
  let seed = 7;
  const rnd = () => {
    seed = (seed * 16807) % 2147483647;
    return seed / 2147483647;
  };
  const pts: [number, number][][] = [];
  for (let r = 0; r <= ROWS; r++) {
    const row: [number, number][] = [];
    for (let c = 0; c <= COLS; c++) {
      const edgeX = c === 0 || c === COLS;
      const edgeY = r === 0 || r === ROWS;
      row.push([(c / COLS) * W + (edgeX ? 0 : (rnd() - 0.5) * 70), (r / ROWS) * H + (edgeY ? 0 : (rnd() - 0.5) * 70)]);
    }
    pts.push(row);
  }
  const shade = (x: number, y: number) => {
    const t = Math.min(1, Math.max(0, (x / W) * 0.75 + (y / H) * 0.25));
    const hue = 168 + t * 24; // 168 (teal) → 192 (blue-teal)
    const light = 16 + (1 - Math.abs(t - 0.45)) * 6 + (rnd() - 0.5) * 4;
    return `hsl(${hue.toFixed(0)} ${48 + t * 12}% ${light.toFixed(1)}%)`;
  };
  const tris: { d: string; fill: string }[] = [];
  for (let r = 0; r < ROWS; r++) {
    for (let c = 0; c < COLS; c++) {
      const [a, b, cc, d] = [pts[r][c], pts[r][c + 1], pts[r + 1][c], pts[r + 1][c + 1]];
      for (const t of rnd() > 0.5 ? [[a, b, d], [a, d, cc]] : [[a, b, cc], [b, d, cc]]) {
        const cx = (t[0][0] + t[1][0] + t[2][0]) / 3;
        const cy = (t[0][1] + t[1][1] + t[2][1]) / 3;
        tris.push({ d: `M${t.map((p) => p.map((v) => v.toFixed(1)).join(',')).join('L')}Z`, fill: shade(cx, cy) });
      }
    }
  }
  return (
    <svg className="lowpoly" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="xMidYMid slice" aria-hidden>
      {tris.map((t, i) => (
        <path key={i} d={t.d} fill={t.fill} stroke="rgba(255,255,255,0.05)" strokeWidth="1" />
      ))}
    </svg>
  );
});
