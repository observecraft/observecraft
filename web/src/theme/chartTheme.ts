import * as echarts from 'echarts';
import { shades } from './shades';

/** Reads a CSS custom property from :root so charts stay in sync with tokens.css. */
export function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export const STATUS_COLORS = {
  error: '--status-error',
  warn: '--status-warn',
  info: '--status-info',
  ok: '--status-ok',
  debug: '--status-debug',
} as const;

export function statusColor(status: keyof typeof STATUS_COLORS): string {
  return token(STATUS_COLORS[status]);
}

export function palette(): string[] {
  return [1, 2, 3, 4, 5, 6, 7, 8].map((i) => token(`--chart-${i}`));
}

/** Single-hue palettes and the chart token they are shades of. */
const HUES: Record<string, number> = { blue: 1, purple: 2, orange: 4, red: 5, green: 6, gray: 8, grey: 8 };

/**
 * Colors for a dashboard request's palette name (integrations-core dashboards use
 * dog_classic, cool, warm and single hues), drawn from our chart tokens.
 */
export function seriesPalette(name?: string): string[] {
  const colors = palette();
  if (name && HUES[name]) return shades(colors[HUES[name] - 1], 6);
  if (name === 'cool') return [colors[0], colors[2], colors[1], ...shades(colors[0], 4).slice(1)];
  if (name === 'warm') return [colors[3], colors[4], colors[6], ...shades(colors[3], 4).slice(1)];
  return colors;
}

let registered = false;

/**
 * Registers the "observecraft" ECharts theme: thin dashed-free gridlines, small
 * gray axis labels, no axis lines on Y, compact tooltips — the dense look of
 * Datadog-style timeseries widgets.
 */
export function registerChartTheme(): void {
  if (registered) return;
  registered = true;

  const axisLabel = { color: token('--chart-axis'), fontSize: 11, fontFamily: token('--font-sans') };
  const grid = token('--chart-grid');

  echarts.registerTheme('observecraft', {
    color: palette(),
    backgroundColor: 'transparent',
    textStyle: { fontFamily: token('--font-sans') },
    grid: { left: 36, right: 12, top: 10, bottom: 22, containLabel: false },
    categoryAxis: {
      axisLine: { show: true, lineStyle: { color: token('--border-strong') } },
      axisTick: { show: false },
      axisLabel,
      splitLine: { show: false },
    },
    timeAxis: {
      axisLine: { show: true, lineStyle: { color: token('--border-strong') } },
      axisTick: { show: false },
      axisLabel,
      splitLine: { show: false },
    },
    valueAxis: {
      axisLine: { show: false },
      axisTick: { show: false },
      axisLabel,
      splitLine: { show: true, lineStyle: { color: grid, width: 1 } },
    },
    line: {
      symbol: 'none',
      lineStyle: { width: 1.5 },
      emphasis: { lineStyle: { width: 2 } },
    },
    bar: { barMaxWidth: 14 },
    tooltip: {
      backgroundColor: token('--bg'),
      borderColor: token('--border'),
      textStyle: { color: token('--text'), fontSize: 12 },
      extraCssText: `box-shadow: ${token('--shadow-pop')}; border-radius: 4px;`,
    },
    legend: {
      textStyle: { color: token('--text-muted'), fontSize: 11 },
      icon: 'roundRect',
      itemWidth: 10,
      itemHeight: 3,
    },
  });
}
