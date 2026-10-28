// Pure helpers over the widget tree (groups contain widgets).
import type { TemplateVariable, TemplateVarValues, Widget, WidgetDefinition, WidgetLayout, WidgetType } from '../../api/dashboards/types';

export function newWidgetId(): string {
  return `w${Date.now().toString(36)}${Math.floor(Math.random() * 1e6).toString(36)}`;
}

/** Applies `fn` to the widget with `id`, wherever it is in the tree. */
export function updateWidget(widgets: Widget[], id: string, fn: (w: Widget) => Widget): Widget[] {
  return widgets.map((w) => {
    if (w.id === id) return fn(w);
    if (w.definition.type === 'group') return { ...w, definition: { ...w.definition, widgets: updateWidget(w.definition.widgets, id, fn) } };
    return w;
  });
}

export function removeWidget(widgets: Widget[], id: string): Widget[] {
  return widgets.filter((w) => w.id !== id).map((w) => (w.definition.type === 'group' ? { ...w, definition: { ...w.definition, widgets: removeWidget(w.definition.widgets, id) } } : w));
}

/** Deep copy with fresh ids, placed directly below the original. */
export function cloneWidget(widgets: Widget[], id: string): Widget[] {
  const out: Widget[] = [];
  for (const w of widgets) {
    if (w.id === id) {
      const copy = reId(structuredClone(w));
      out.push(w, { ...copy, layout: { ...w.layout, y: w.layout.y + w.layout.h } });
    } else if (w.definition.type === 'group') {
      out.push({ ...w, definition: { ...w.definition, widgets: cloneWidget(w.definition.widgets, id) } });
    } else out.push(w);
  }
  return out;
}

function reId(w: Widget): Widget {
  const def = w.definition.type === 'group' ? { ...w.definition, widgets: w.definition.widgets.map(reId) } : w.definition;
  return { ...w, id: newWidgetId(), definition: def };
}

/** Applies a grid layout change (from the grid library) to one level of the tree. */
export function applyLayouts(widgets: Widget[], layouts: { i: string; x: number; y: number; w: number; h: number }[]): Widget[] {
  const byId = new Map(layouts.map((l) => [l.i, l]));
  return widgets.map((w) => {
    const l = byId.get(w.id);
    return l ? { ...w, layout: { x: l.x, y: l.y, w: l.w, h: w.definition.type === 'group' ? w.layout.h : l.h } } : w;
  });
}

export function bottom(widgets: Widget[]): number {
  return widgets.reduce((m, w) => Math.max(m, w.layout.y + w.layout.h), 0);
}

/** Group height follows its content: header (2 rows, 1 compact, none untitled) + children + padding. */
export function groupHeight(children: Widget[], compact = false, titled = true): number {
  return (titled ? (compact ? 2 : 3) : 1) + Math.max(2, bottom(children));
}

export const DEFAULT_SIZE: Record<WidgetType, Pick<WidgetLayout, 'w' | 'h'>> = {
  integration: { w: 6, h: 6 },
  timeseries: { w: 6, h: 6 },
  query_value: { w: 3, h: 4 },
  toplist: { w: 4, h: 6 },
  table: { w: 12, h: 7 },
  bar_chart: { w: 6, h: 6 },
  change: { w: 6, h: 6 },
  heatmap: { w: 6, h: 6 },
  distribution: { w: 6, h: 6 },
  pie: { w: 4, h: 6 },
  treemap: { w: 6, h: 6 },
  note: { w: 4, h: 4 },
  image: { w: 4, h: 5 },
  group: { w: 12, h: 6 },
  monitor_summary: { w: 6, h: 10 },
  log_stream: { w: 12, h: 8 },
};

const Q = 'avg:system.cpu.user{*}';
const QBY = 'avg:system.cpu.user{*} by {host}';

/** Starting definition when a widget is added from the tray. */
export function defaultDefinition(type: WidgetType): WidgetDefinition {
  switch (type) {
    case 'integration':
      return { type, title: 'Integration panel', original: { type: 'timeseries', requests: [] } };
    case 'timeseries':
      return { type, title: '', requests: [{ query: QBY }], display: 'line' };
    case 'query_value':
      return { type, title: '', request: { query: Q }, aggregator: 'avg', precision: 1, formats: [] };
    case 'toplist':
      return { type, title: '', request: { query: QBY }, aggregator: 'avg', limit: 10 };
    case 'change':
      return { type, title: '', request: { query: QBY }, aggregator: 'avg', limit: 10 };
    case 'bar_chart':
      return { type, title: '', request: { query: 'avg:system.cpu.user{*} by {kube_cluster_name}' }, aggregator: 'avg', limit: 10 };
    case 'table':
      return {
        type,
        title: '',
        requests: [
          { query: 'sum:trace.http.request.hits{*} by {service}', alias: 'Requests' },
          { query: 'avg:trace.http.request.duration.p95{*} by {service}', alias: 'p95 (ms)' },
        ],
        aggregator: 'avg',
        limit: 25,
        searchable: true,
      };
    case 'heatmap':
    case 'distribution':
      return { type, title: '', request: { query: Q } };
    case 'pie':
    case 'treemap':
      return { type, title: '', request: { query: 'avg:system.mem.used{*} by {kube_cluster_name}' }, aggregator: 'avg' };
    case 'note':
      return { type, content: '## New note\n\nWrite **markdown** here. Links: [ObserveCraft](https://example.com)', background: 'yellow', align: 'left', fontSize: 'md' };
    case 'image':
      return { type, logo: 'kubernetes', background: 'linear-gradient(135deg,#1fb6ff,#0aa2e8)' };
    case 'group':
      return { type, title: 'New group', color: 'blue', widgets: [] };
    case 'monitor_summary':
      return { type, title: 'Monitor status', query: '' };
    case 'log_stream':
      return { type, title: 'Logs', query: 'status:error', limit: 15 };
  }
}

/** A log search as dashboard widgets (Log Explorer → Open in Dashboards). */
export function logStreamWidgets(query: string): Widget[] {
  return [{ id: newWidgetId(), definition: { type: 'log_stream', title: query.trim() || 'All logs', query, limit: 50 }, layout: { x: 0, y: 0, ...DEFAULT_SIZE.log_stream } }];
}

/** Template variables shown before "+N": the first five and any later one in use. */
export function visibleVariables(vars: TemplateVariable[], values: TemplateVarValues, expanded: boolean, limit = 5): { shown: TemplateVariable[]; hidden: number } {
  // Folding a single variable behind "+1" saves no room.
  if (expanded || vars.length <= limit + 1) return { shown: vars, hidden: 0 };
  const shown = vars.filter((v, i) => i < limit || (values[v.name] !== undefined && values[v.name] !== v.default));
  return { shown, hidden: vars.length - shown.length };
}
