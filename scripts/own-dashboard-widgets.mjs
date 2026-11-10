// Widgets of ObserveCraft's own integration dashboards (authentik, FleetDM): queries and widgets in the Datadog widget
// format the integration dashboards use, converted like them (import-integration-dashboards.mjs).
import { convertDashboard } from './import-integration-dashboards.mjs';

// ---------------------------------------------------------------- queries (Datadog format)

export const metric = (query, aggregator = 'last', name = 'query1') => ({ data_source: 'metrics', name, query, aggregator });

/** A log query: a count, or another aggregation of a facet (`cardinality` of `@usr.name`); grouped by facets. */
export const logs = (search, { name = 'query1', groupBy = [], limit = 10, aggregation = 'count', metric: facet } = {}) => ({
  data_source: 'logs',
  name,
  search: { query: search },
  indexes: ['*'],
  compute: { aggregation, ...(facet ? { metric: facet } : {}) },
  group_by: groupBy.map((f) => ({ facet: f, limit, sort: { aggregation: 'count', order: 'desc' } })),
  storage: 'hot',
});

/** A Cloud SIEM signal count (the signal search), grouped by a facet (`status`, `@workflow.rule.name`). */
export const signals = (search, { name = 'query1', groupBy = [] } = {}) => ({
  data_source: 'security_signals',
  name,
  search: { query: search },
  indexes: ['*'],
  compute: { aggregation: 'count' },
  group_by: groupBy.map((f) => ({ facet: f, limit: 10, sort: { aggregation: 'count', order: 'desc' } })),
});

/** `@evt.name:(a OR b)`. */
export const anyOf = (types, attribute = '@evt.name') => `${attribute}:(${types.join(' OR ')})`;

// ---------------------------------------------------------------- widgets (Datadog format)

const head = (title) => ({ title, title_size: '16', title_align: 'left' });

export function queryValue(title, queries, { formula = 'query1', formats = [], timeseries = false, unit, precision = 0 } = {}) {
  return {
    type: 'query_value',
    ...head(title),
    requests: [{ response_format: 'scalar', queries, formulas: [{ formula }], conditional_formats: formats }],
    autoscale: true,
    precision,
    ...(unit ? { custom_unit: unit } : {}),
    ...(timeseries ? { timeseries_background: { type: 'bars' } } : {}),
  };
}

export function timeseries(title, queries, { formulas, display = 'line', palette = 'dog_classic' } = {}) {
  return {
    type: 'timeseries',
    ...head(title),
    show_legend: true,
    legend_layout: 'auto',
    requests: [{ response_format: 'timeseries', queries, formulas: formulas ?? queries.map((q) => ({ formula: q.name })), style: { palette, order_by: 'values' }, display_type: display }],
    yaxis: { include_zero: true },
  };
}

export function toplist(title, query, { count = 10, palette, order = 'desc' } = {}) {
  return {
    type: 'toplist',
    ...head(title),
    requests: [
      { response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }], sort: { count, order_by: [{ type: 'formula', index: 0, order }] }, ...(palette ? { style: { palette } } : {}) },
    ],
  };
}

/** Columns: [alias, query, aggregator] (metric queries, or log queries as they are); rows: the first query's groups. */
export function table(title, columns, count = 100) {
  const queries = columns.map(([, query, aggregator], i) => (typeof query === 'string' ? metric(query, aggregator ?? 'last', `query${i + 1}`) : { ...query, name: `query${i + 1}` }));
  return {
    type: 'query_table',
    ...head(title),
    requests: [
      { response_format: 'scalar', queries, formulas: columns.map(([alias], i) => ({ alias, formula: `query${i + 1}` })), sort: { count, order_by: [{ type: 'formula', index: 0, order: 'desc' }] } },
    ],
  };
}

/** A world map of the query's groups by country (ISO code), in Datadog's `hostmap_blues` scale. */
export const geomap = (title, query) => ({
  type: 'geomap',
  ...head(title),
  requests: [{ response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }], sort: { count: 250, order_by: [{ type: 'formula', index: 0, order: 'desc' }] } }],
  style: { palette: 'hostmap_blues', palette_flip: false },
  view: { focus: 'WORLD' },
});

/** A donut of the query's groups, one color each, with their share in the legend. */
export const pie = (title, queries, formulas = [{ formula: 'query1' }]) => ({
  type: 'sunburst',
  ...head(title),
  requests: [{ response_format: 'scalar', queries, formulas, sort: { count: 12, order_by: [{ type: 'formula', index: 0, order: 'desc' }] }, style: { palette: 'dog_classic' } }],
  legend: { type: 'inline' },
  hide_total: false,
});

/** Rectangles sized by value, one color per group. */
export const treemap = (title, query) => ({ type: 'treemap', ...head(title), requests: [{ response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }] }] });

/** Vertical bars, one color per group. */
export const bars = (title, query, count = 10) => ({
  type: 'bar_chart',
  ...head(title),
  requests: [
    { response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }], sort: { count, order_by: [{ type: 'formula', index: 0, order: 'desc' }] }, style: { palette: 'dog_classic' } },
  ],
});

export const logStream = (title, query) => ({
  type: 'log_stream',
  title,
  query,
  columns: ['host', 'service'],
  indexes: [],
  message_display: 'inline',
  show_date_column: true,
  show_message_column: true,
  sort: { column: 'time', order: 'desc' },
});

export const note = (content, background_color = 'white', { align = 'left', size = '14' } = {}) => ({
  type: 'note',
  content,
  background_color,
  font_size: size,
  text_align: align,
  show_tick: false,
  tick_pos: '50%',
  tick_edge: 'left',
  has_padding: true,
});

export const GOOD = [{ comparator: '>', value: 0, palette: 'green_on_white' }];
export const BAD = [{ comparator: '>', value: 0, palette: 'red_on_white' }];
export const WARN = [{ comparator: '>', value: 0, palette: 'yellow_on_white' }];
export const NONE = [{ comparator: '>=', value: 0, palette: 'white_on_gray' }];

// ---------------------------------------------------------------- layout

/** Widget placement with ids numbered from 1 for each dashboard. */
export function layout() {
  let seq = 0;
  const at = (definition, x, y, width, height) => ({ id: ++seq, definition, layout: { x, y, width, height } });
  const group = (title, color, widgets, y, height, extra = {}) => at({ type: 'group', title, background_color: color, layout_type: 'ordered', widgets, ...extra }, 0, y, 12, height);
  return { at, group };
}

/** The converted dashboard `id` (`<integration>-overview`), with our logo tile instead of a banner image (there is
 * none for these vendors), as the integrations-core dashboards have. */
export function buildOwnDashboard(doc, { integration, name, script }) {
  const withLogo = (widgets) =>
    widgets.map((w) => {
      if (w.definition.type === 'group') return { ...w, definition: { ...w.definition, widgets: withLogo(w.definition.widgets) } };
      if (w.definition.type === 'image') return { ...w, definition: { type: 'image', title: name, logo: integration, background: '#ffffff' } };
      return w;
    });
  const renamed = (widgets) =>
    widgets.map((w) => ({
      ...w,
      id: w.id.replace(/^upstream-/, `${integration}-`),
      definition: w.definition.type === 'group' ? { ...w.definition, widgets: renamed(w.definition.widgets) } : w.definition,
    }));
  const converted = convertDashboard(doc, integration, `${integration}_overview.json`, 'observecraft');
  return {
    ...converted,
    id: `${integration}-overview`,
    description: doc.description,
    widgets: renamed(withLogo(converted.widgets)),
    source: { repository: 'observecraft', path: script, license: 'MIT' },
  };
}
