import type { IntegrationWidgetDefinition } from './evaluate';

/** The metric panels the renderer draws (IntegrationMetric). */
export const METRIC_PANELS = ['timeseries', 'query_value', 'query_table', 'toplist', 'sunburst', 'treemap', 'check_status', 'change', 'heatmap', 'geomap', 'bar_chart', 'distribution'];

/** Lists drawn from other data than metrics. */
const LISTS = ['infrastructure', 'logs_pattern_stream', 'event_stream'];

/** Whether an upstream widget is drawn as an event list (IntegrationEvents) rather than a metric panel. */
export function isEventPanel(def: IntegrationWidgetDefinition): boolean {
  return def.type === 'list_stream' || def.type === 'event_stream' || def.type === 'event_timeline' || !!def.requests?.some?.((r) => r.queries?.some((q) => q.data_source === 'events'));
}

/** Why the renderer can't draw an upstream widget, or null when it can. */
export function panelSupport(def: IntegrationWidgetDefinition): string | null {
  if (def.type === 'free_text' || def.type === 'hostmap' || def.type === 'topology_map') return null;
  const stream = def.requests?.[0]?.query?.data_source;
  if (def.type === 'list_stream' && stream && !LISTS.includes(stream)) return `The ${stream.replace(/_/g, ' ')} list is not supported yet`;
  if (isEventPanel(def)) return null;
  return METRIC_PANELS.includes(def.type) ? null : `The ${def.type} panel requires a data source that is not supported yet.`;
}
