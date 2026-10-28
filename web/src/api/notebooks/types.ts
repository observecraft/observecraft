// Notebooks wire contract. A notebook is a document of ordered cells: Markdown text and live query
// cells whose definitions have the shape of dashboard widget definitions (../dashboards/types), so
// they render with the dashboard widgets and can be exported to a dashboard. The server checks the
// cell types and sizes (server/crates/oc-api/src/routes/notebooks.rs).
import type { components } from '../generated/schema';
import type { WidgetDefinition } from '../dashboards/types';

type S = components['schemas'];

/** The relative time ranges of the time picker (components/shell/TimeContext.tsx). */
export type NotebookTime = '15m' | '1h' | '4h' | '1d' | '2d' | '1w';
export const NOTEBOOK_TIMES: NotebookTime[] = ['15m', '1h', '4h', '1d', '2d', '1w'];

export type MarkdownCellDefinition = { type: 'markdown'; text: string };
type QueryCellDefinition = Extract<WidgetDefinition, { type: 'timeseries' | 'query_value' | 'toplist' | 'log_stream' }>;
export type CellDefinition = MarkdownCellDefinition | QueryCellDefinition;
export type CellType = CellDefinition['type'];
export const CELL_TYPES: CellType[] = ['markdown', 'timeseries', 'query_value', 'toplist', 'log_stream'];

export interface NotebookCell {
  id: string;
  definition: CellDefinition;
  /** Overrides the notebook's time range for this cell. */
  time?: NotebookTime | null;
}

export type NotebookStatus = S['NotebookStatus'];
export type NotebookAuthor = S['NotebookAuthor'];
export type NotebookListView = S['NotebookListView'];
export type NotebookSortField = S['NotebookSortField'];
export type NotebookSearchRequest = S['NotebookSearchRequest'];

export interface NotebookSummary {
  id: string;
  title: string;
  author: NotebookAuthor | null;
  teams: string[];
  tags: string[];
  status: NotebookStatus;
  time: NotebookTime;
  cellCount: number;
  createdAt: number;
  modifiedAt: number;
  viewedAt: number | null;
  favorite: boolean;
  deletedAt: number | null;
}

export interface Notebook extends NotebookSummary {
  cells: NotebookCell[];
}

export interface NotebookSearchPage {
  notebooks: NotebookSummary[];
  total: number;
  authors: NotebookAuthor[];
  teams: string[];
}

export interface CreateNotebookRequest {
  title: string;
  teams?: string[];
  tags?: string[];
  status?: NotebookStatus;
  time?: NotebookTime;
  cells?: NotebookCell[];
}

export interface NotebookPatch {
  title?: string;
  teams?: string[];
  tags?: string[];
  status?: NotebookStatus;
  time?: NotebookTime;
  cells?: NotebookCell[];
}
