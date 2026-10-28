// Sheets (Dashboards → Sheets) wire contract. A spreadsheet holds tabs ("sheets"); each is backed by
// a data source query, run in the browser through the existing metric, log and reference table APIs,
// plus calculated columns, filters, a pivot and a sort (see lib/sheetGrid.ts). The server stores the
// definition as opaque JSON (server/crates/oc-api/src/routes/sheets.rs checks only its shape).
import type { TimeAggregator } from '../dashboards/types';
import type { CalculatedColumn, SheetFilter, SheetPivot, SheetSort } from '../../lib/sheetGrid';

export type SheetTime = '15m' | '1h' | '4h' | '1d' | '2d' | '1w';
export const SHEET_TIMES: SheetTime[] = ['15m', '1h', '4h', '1d', '2d', '1w'];

export type SheetSource =
  /** One row per group of the query; one column per tag of `by` and per aggregator. */
  | { type: 'metrics'; query: string; aggregators: TimeAggregator[] }
  /** Grouped counts of the matching logs by `groupBy`, or (empty `groupBy`) the newest matching logs. */
  | { type: 'logs'; query: string; groupBy: string; limit: number }
  | { type: 'reference_table'; table: string }
  | { type: 'none' };

export interface Sheet {
  id: string;
  name: string;
  time: SheetTime;
  source: SheetSource;
  calculated: CalculatedColumn[];
  filters: SheetFilter[];
  pivot: SheetPivot | null;
  sort: SheetSort | null;
}

export interface SpreadsheetDefinition {
  sheets: Sheet[];
}

export interface UserRef {
  id: string;
  name: string;
}

export interface SpreadsheetSummary {
  id: string;
  name: string;
  author: UserRef | null;
  team: string | null;
  sheetCount: number;
  createdAt: number;
  modifiedAt: number;
}

export interface Spreadsheet extends SpreadsheetSummary {
  definition: SpreadsheetDefinition;
}

export type SpreadsheetView = 'all' | 'mine';

export interface SpreadsheetSearchRequest {
  query: string;
  view: SpreadsheetView;
  team: string | null;
}

export interface SpreadsheetSearchPage {
  spreadsheets: SpreadsheetSummary[];
  total: number;
  teams: string[];
}

export interface CreateSpreadsheetRequest {
  name: string;
  team?: string | null;
  definition?: SpreadsheetDefinition;
}

export interface SpreadsheetPatch {
  name?: string;
  /** "" clears the team. */
  team?: string;
  definition?: SpreadsheetDefinition;
}

export const newSheet = (n: number): Sheet => ({
  id: `sheet${n}-${Date.now().toString(36)}`,
  name: `Sheet ${n}`,
  time: '1h',
  source: { type: 'none' },
  calculated: [],
  filters: [],
  pivot: null,
  sort: null,
});

/** Fills in what older or hand-written definitions leave out. */
export function normalizeSheet(s: Partial<Sheet> & Pick<Sheet, 'id' | 'name'>): Sheet {
  return { time: '1h', source: { type: 'none' }, calculated: [], filters: [], pivot: null, sort: null, ...s };
}
