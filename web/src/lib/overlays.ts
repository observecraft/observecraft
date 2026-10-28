// Event overlays on dashboard timeseries: events as vertical markers over the series.
import type { LineSeriesOption } from 'echarts';

export interface OverlayEvent {
  timestamp: number;
  title: string;
  /** error | warning | info | success */
  alertType: string;
}

export interface OverlayMark {
  /** Start of the minute. */
  t: number;
  /** The worst status of the minute's events. */
  status: string;
  titles: string[];
}

const RANK: Record<string, number> = { error: 3, warning: 2, info: 1, success: 0 };
/** Markers drawn per chart at most (the most severe first). */
const MAX_MARKS = 100;

/** Events grouped by minute, oldest first. */
export function overlayMarks(events: OverlayEvent[]): OverlayMark[] {
  const byMinute = new Map<number, OverlayMark>();
  for (const e of events) {
    const t = Math.floor(e.timestamp / 60_000) * 60_000;
    const m = byMinute.get(t) ?? { t, status: e.alertType, titles: [] };
    // The most severe event's title first: it names the marker.
    if ((RANK[e.alertType] ?? 1) > (RANK[m.status] ?? 1)) {
      m.status = e.alertType;
      m.titles.unshift(e.title);
    } else m.titles.push(e.title);
    byMinute.set(t, m);
  }
  const marks = [...byMinute.values()].sort((a, b) => (RANK[b.status] ?? 1) - (RANK[a.status] ?? 1) || b.t - a.t).slice(0, MAX_MARKS);
  return marks.sort((a, b) => a.t - b.t);
}

/** The status token of a marker's colour. */
export function overlayStatusToken(status: string): string {
  return status === 'error' ? '--status-error' : status === 'warning' ? '--status-warn' : '--status-info';
}

/** An ECharts series that only draws the marks (dashed vertical lines, the titles on hover). */
export function overlaySeries(marks: OverlayMark[], color: (status: string) => string): LineSeriesOption & { markLine: { data: { xAxis: number; name: string; lineStyle: { color: string } }[] } } {
  return {
    // No name: overlays stay out of the chart's legend.
    name: '',
    type: 'line' as const,
    data: [] as [number, number][],
    silent: false,
    markLine: {
      symbol: 'none',
      label: { show: false },
      emphasis: { label: { show: true, formatter: '{b}', position: 'insideEndTop' as const, fontSize: 11 } },
      data: marks.map((m) => ({
        xAxis: m.t,
        name: m.titles.length > 1 ? `${m.titles[0]} (+${m.titles.length - 1} more)` : m.titles[0],
        lineStyle: { color: color(m.status), type: 'dashed' as const, width: 1 },
      })),
    },
  };
}
