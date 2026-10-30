// Report widgets: a big number, a donut with its legend table, a treemap of counts and a titled section.
import type { ReactNode } from 'react';
import type { EChartsOption } from 'echarts';
import { Chart } from '../../../components/charts/Chart';
import { palette } from '../../../theme/chartTheme';
import { squarify } from '../../metrics/squarify';

/** 8230 → "8.23k", as the report tiles read. */
export function short(n: number): string {
  if (Math.abs(n) >= 1e6) return `${+(n / 1e6).toPrecision(3)}M`;
  if (Math.abs(n) >= 1e3) return `${+(n / 1e3).toPrecision(3)}k`;
  return n.toLocaleString('en-US');
}

/** A report section: a heading and its widgets (`id` is what the report navigation scrolls to). */
export function ReportSection({ id, title, banner, children }: { id: string; title: string; banner?: boolean; children: ReactNode }) {
  return (
    <section id={id} className={`rw-section${banner ? ' rw-banner' : ''}`}>
      <h2>{title}</h2>
      {children}
    </section>
  );
}

/** A tile with a title and a number that fills it. */
export function BigNumber({ title, value, format = short, tone }: { title: string; value: number | null | undefined; format?: (n: number) => string; tone?: string }) {
  const text = value === null || value === undefined ? '—' : format(value);
  return (
    <div className="rw-tile">
      <span className="rw-tile-title">{title}</span>
      <strong className="rw-big" style={{ fontSize: `min(62cqh, ${Math.round(150 / Math.max(1.6, text.length))}cqw)`, color: tone }}>
        {text}
      </strong>
    </div>
  );
}

export interface Slice {
  label: string;
  value: number;
}

/** A donut with the total in its middle, and a table of its slices with their counts and shares. */
export function DonutWithLegend({ title, slices }: { title: string; slices: Slice[] }) {
  const colors = palette();
  const total = slices.reduce((n, s) => n + s.value, 0);
  const sorted = [...slices].sort((a, b) => b.value - a.value);
  const option: EChartsOption = {
    tooltip: { trigger: 'item', formatter: (p) => `${(p as { name: string }).name}: ${short(Number((p as { value: unknown }).value))}` },
    series: [
      {
        type: 'pie',
        radius: ['42%', '70%'],
        avoidLabelOverlap: true,
        label: { formatter: (p) => `${p.name}\n{v|${short(Number(p.value))} • ${Number(p.percent).toFixed(1)}%}`, rich: { v: { fontWeight: 700 } } },
        data: sorted.map((s) => ({ name: s.label, value: s.value, itemStyle: { color: colors[slices.indexOf(s) % colors.length] } })),
      },
    ],
    graphic: [{ type: 'text', left: 'center', top: 'middle', style: { text: short(total), fontSize: 28, fontWeight: 600 } }],
  };
  return (
    <div className="rw-card rw-donut">
      <div className="rw-card-title">{title}</div>
      <div className="rw-donut-body">
        <div className="rw-donut-chart">
          <Chart option={option} height="100%" />
        </div>
        <table className="rw-legend">
          <thead>
            <tr>
              <th />
              <th>QUERY LABEL</th>
              <th className="num">COUNT</th>
              <th className="num">SHARE</th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((s) => (
              <tr key={s.label}>
                <td>
                  <i style={{ background: colors[slices.indexOf(s) % colors.length] }} />
                </td>
                <td>{s.label}</td>
                <td className="num">{short(s.value)}</td>
                <td className="num">{total ? `${((s.value * 100) / total).toFixed(2)} %` : '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

/** Counts as a treemap: a "Total" bar, then a box per value sized by its count. */
export function CountTreemap({ title, slices }: { title: string; slices: Slice[] }) {
  const colors = palette();
  const [W, H] = [1000, 700];
  const shown = slices.filter((s) => s.value > 0);
  const total = shown.reduce((n, s) => n + s.value, 0);
  return (
    <div className="rw-card rw-treemap">
      <div className="rw-card-title">{title}</div>
      <div className="rw-treemap-total">
        <span>Total</span>
        <strong>{short(total)}</strong>
      </div>
      <div className="rw-treemap-body">
        {shown.length ? (
          squarify(shown, (s) => s.value, W, H).map(({ item, x, y, w, h }) => {
            const size = Math.max(10, Math.min(56, Math.sqrt(w * h) / 6, w / 5));
            return (
              <div
                key={item.label}
                className="rw-tile-box"
                title={`${item.label}: ${item.value.toLocaleString('en-US')}`}
                style={{ left: `${(x / W) * 100}%`, top: `${(y / H) * 100}%`, width: `${(w / W) * 100}%`, height: `${(h / H) * 100}%`, background: colors[slices.indexOf(item) % colors.length] }}
              >
                <span style={{ fontSize: size * 0.6 }}>{item.label}</span>
                <strong style={{ fontSize: size }}>{short(item.value)}</strong>
              </div>
            );
          })
        ) : (
          <p className="pr-muted rw-empty">No data</p>
        )}
      </div>
    </div>
  );
}
