// Dashboards wire contract. A dashboard is a JSON document (layout + template variables +
// widget definitions); widgets carry metric queries that the query service evaluates.
import type { MetricSeries, TimeRange } from '../types';

// ---------------------------------------------------------------- metric queries

/** `avg:kubernetes_state.deployment.replicas{kube_cluster_name:a,$namespace} by {kube_deployment}` */
export type MetricQueryString = string;
export type SpaceAggregator = 'avg' | 'sum' | 'min' | 'max';
export type TimeAggregator = 'avg' | 'sum' | 'min' | 'max' | 'last';

export interface MetricRequest {
  query: MetricQueryString;
  alias?: string;
}

/** Template variable values applied to queries: name → value ("*" = all). */
export type TemplateVarValues = Record<string, string>;

/** Queries arrive with template variables already resolved by the client. */
export interface TimeseriesRequest {
  queries: MetricQueryString[];
  range: TimeRange;
  /** Rough number of points wanted per series. */
  points?: number;
}

export interface ScalarRequest {
  query: MetricQueryString;
  aggregator: TimeAggregator;
  range: TimeRange;
}

/** One value per group ("by" tags). Used by top lists, tables, pies, treemaps. */
export interface GroupedScalar {
  group: string; // "kube_cluster_name:a, kube_namespace:b" or "*"
  tags: Record<string, string>;
  value: number;
}

export interface DistributionResponse {
  /** Bucket upper bounds and counts per time slot, for heatmaps / distributions. */
  buckets: number[];
  times: number[];
  counts: number[][]; // [timeIndex][bucketIndex]
}

export interface MetricCatalogEntry {
  name: string;
  tags: string[];
  unit?: string;
  description: string;
}

// ---------------------------------------------------------------- widgets

export type ConditionalPalette = 'green' | 'yellow' | 'red' | 'gray' | 'blue';

export interface ConditionalFormat {
  comparator: '>' | '>=' | '<' | '<=';
  value: number;
  palette: ConditionalPalette;
}

interface Base {
  title?: string;
}

export type WidgetDefinition =
  | (Base & { type: 'timeseries'; requests: MetricRequest[]; display: 'line' | 'area' | 'bars'; stacked?: boolean; yMax?: number })
  | (Base & { type: 'query_value'; request: MetricRequest; aggregator: TimeAggregator; precision: number; unit?: string; formats: ConditionalFormat[]; fill?: boolean })
  | (Base & { type: 'toplist'; request: MetricRequest; aggregator: TimeAggregator; limit: number; showChange?: boolean })
  /** Columns = requests; rows = groups of the first request's `by` tags. */
  | (Base & { type: 'table'; requests: MetricRequest[]; aggregator: TimeAggregator; limit: number; searchable?: boolean })
  | (Base & { type: 'bar_chart'; request: MetricRequest; aggregator: TimeAggregator; limit: number })
  /** Value per group plus change vs. the previous period of equal length. */
  | (Base & { type: 'change'; request: MetricRequest; aggregator: TimeAggregator; limit: number })
  | (Base & { type: 'heatmap'; request: MetricRequest })
  | (Base & { type: 'distribution'; request: MetricRequest })
  | (Base & { type: 'pie'; request: MetricRequest; aggregator: TimeAggregator })
  | (Base & { type: 'treemap'; request: MetricRequest; aggregator: TimeAggregator })
  | (Base & { type: 'note'; content: string; background: 'white' | 'yellow' | 'blue' | 'purple' | 'gray' | 'green' | 'red'; align: 'left' | 'center'; fontSize: 'sm' | 'md' | 'lg' | 'xl'; pointer?: boolean })
  | (Base & { type: 'image'; src?: string; logo?: string; background?: string })
  | (Base & { type: 'group'; color: 'blue' | 'purple' | 'green' | 'gray' | 'orange'; widgets: Widget[] })
  | (Base & { type: 'monitor_summary'; query: string })
  | (Base & { type: 'log_stream'; query: string; limit: number });

export type WidgetType = WidgetDefinition['type'];

/** Grid units: 12 columns; rows are 40px. Free layout uses a finer grid (see Dashboard.layoutType). */
export interface WidgetLayout {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Widget {
  id: string;
  definition: WidgetDefinition;
  layout: WidgetLayout;
}

export interface TemplateVariable {
  name: string; // "kube_cluster_name"
  prefix: string; // tag key it filters on
  default: string; // "*"
}

/** grid = snap to 12-col grid; auto = timeboard (uniform flow); free = screenboard (fine grid, overlap allowed). */
export type LayoutType = 'grid' | 'auto' | 'free';

export interface User {
  id: string;
  name: string;
}

export interface DashboardSummary {
  id: string;
  title: string;
  description?: string;
  kind: 'custom' | 'integration';
  /** Integration logo key (see IntegrationLogo) for integration dashboards. */
  logo?: string;
  author: User | null;
  teams: string[];
  popularity: number; // 0..5
  createdAt: number;
  modifiedAt: number;
  viewedAt: number | null;
  starred: boolean;
  deletedAt: number | null;
}

export interface Dashboard extends DashboardSummary {
  layoutType: LayoutType;
  templateVariables: TemplateVariable[];
  widgets: Widget[];
}

export type PresetList = 'all' | 'custom' | 'integrations' | 'mine' | 'starred' | 'deleted';

export interface DashboardSearchRequest {
  query: string;
  preset: PresetList;
  listId: string | null;
  team: string | null;
  sort: { field: 'title' | 'modified' | 'popularity'; dir: 'asc' | 'desc' };
  offset: number;
  limit: number;
}

export interface DashboardSearchPage {
  dashboards: DashboardSummary[];
  total: number;
}

export interface DashboardList {
  id: string;
  name: string;
  dashboardIds: string[];
}

export interface CreateDashboardRequest {
  title: string;
  teams: string[];
  layoutType: LayoutType;
}

export interface MonitorSummary {
  counts: { alert: number; warn: number; ok: number; nodata: number };
  monitors: { id: string; name: string; status: 'alert' | 'warn' | 'ok' | 'nodata'; muted: boolean }[];
}

export type { MetricSeries };
