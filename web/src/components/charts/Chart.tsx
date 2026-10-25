import { useEffect, useRef } from 'react';
import * as echarts from 'echarts';
import type { EChartsOption } from 'echarts';
import { registerChartTheme } from '../../theme/chartTheme';

interface ChartProps {
  option: EChartsOption;
  height: number | string;
  onClick?: (params: echarts.ECElementEvent) => void;
}

/** Thin ECharts wrapper: themed, auto-resizing, option replaced on change. */
export function Chart({ option, height, onClick }: ChartProps) {
  const el = useRef<HTMLDivElement>(null);
  const chart = useRef<echarts.ECharts | null>(null);

  useEffect(() => {
    registerChartTheme();
    const instance = echarts.init(el.current!, 'observecraft', { renderer: 'canvas' });
    chart.current = instance;
    const ro = new ResizeObserver(() => instance.resize());
    ro.observe(el.current!);
    return () => {
      ro.disconnect();
      instance.dispose();
      chart.current = null;
    };
  }, []);

  useEffect(() => {
    // No entry animation: data should appear instantly, like a monitoring tool, not a slideshow.
    chart.current?.setOption({ animation: false, ...option }, { notMerge: true });
  }, [option]);

  useEffect(() => {
    const instance = chart.current;
    if (!instance || !onClick) return;
    instance.on('click', onClick);
    return () => {
      instance.off('click', onClick);
    };
  }, [onClick]);

  return <div ref={el} style={{ width: '100%', height }} />;
}
