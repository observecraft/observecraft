// Sheets data access. In "live" mode spreadsheets are stored by the Rust API (Postgres); otherwise
// the in-browser mock (mock/sheets.ts) answers. A sheet's data comes from the existing metric, log and
// reference table clients, so it works the same in both modes.
import type { components } from '../generated/schema';
import { API_MODE, del, get, patch, post } from '../http';
import { searchLogPage, topLogValues } from '../client';
import { queryGrouped } from '../dashboards/client';
import { getReferenceTable } from '../referenceTables/client';
import type { TimeRange } from '../types';
import * as mock from '../mock/sheets';
import type { GridTable, Row } from '../../lib/sheetGrid';
import {
  normalizeSheet,
  type CreateSpreadsheetRequest,
  type SheetSource,
  type Spreadsheet,
  type SpreadsheetDefinition,
  type SpreadsheetPatch,
  type SpreadsheetSearchPage,
  type SpreadsheetSearchRequest,
} from './types';

export * from './types';

const live = API_MODE === 'live';
type S = components['schemas'];

const toSummary = (s: S['SpreadsheetSummary']) => ({ ...s, author: s.author ?? null, team: s.team ?? null });
const toSpreadsheet = (s: S['Spreadsheet']): Spreadsheet => {
  const def = s.definition as unknown as SpreadsheetDefinition;
  return { ...toSummary(s), definition: { sheets: (def.sheets ?? []).map(normalizeSheet) } };
};
const wire = (d: SpreadsheetDefinition | undefined) => d as unknown as Record<string, never> | undefined;

export async function searchSpreadsheets(req: SpreadsheetSearchRequest): Promise<SpreadsheetSearchPage> {
  if (!live) return mock.searchSpreadsheets(req);
  const page = await post('/api/v1/spreadsheets/search', req);
  return { ...page, spreadsheets: page.spreadsheets.map(toSummary) };
}

export async function getSpreadsheet(id: string): Promise<Spreadsheet> {
  if (!live) return mock.getSpreadsheet(id);
  return toSpreadsheet(await get('/api/v1/spreadsheets/{id}', { id }));
}

export async function createSpreadsheet(req: CreateSpreadsheetRequest): Promise<Spreadsheet> {
  if (!live) return mock.createSpreadsheet(req);
  return toSpreadsheet(await post('/api/v1/spreadsheets', { name: req.name, team: req.team ?? null, definition: wire(req.definition) ?? null }));
}

export async function updateSpreadsheet(id: string, p: SpreadsheetPatch): Promise<Spreadsheet> {
  if (!live) return mock.updateSpreadsheet(id, p);
  return toSpreadsheet(await patch('/api/v1/spreadsheets/{id}', { id }, { name: p.name ?? null, team: p.team ?? null, definition: wire(p.definition) ?? null }));
}

export const deleteSpreadsheet = (id: string): Promise<void> => (live ? del('/api/v1/spreadsheets/{id}', { id }) : mock.deleteSpreadsheet(id));

// ---------------------------------------------------------------- data sources

const LOG_ROWS = 200;

/** Runs a sheet's data source over `range` and returns it as a table. */
export async function loadSheetSource(source: SheetSource, range: TimeRange): Promise<GridTable> {
  switch (source.type) {
    case 'none':
      return { columns: [], rows: [] };
    case 'metrics': {
      const aggregators = source.aggregators.length ? source.aggregators : ['avg' as const];
      const results = await Promise.all(aggregators.map((aggregator) => queryGrouped({ query: source.query, aggregator, range })));
      const tagKeys = [...new Set(results.flatMap((r) => r.flatMap((g) => Object.keys(g.tags))))];
      const byGroup = new Map<string, Row>();
      results.forEach((groups, i) => {
        for (const g of groups) {
          const row = byGroup.get(g.group) ?? Object.fromEntries(tagKeys.map((k) => [k, g.tags[k] ?? null]));
          if (tagKeys.length === 0) row.group = g.group;
          row[aggregators[i]] = g.value;
          byGroup.set(g.group, row);
        }
      });
      const groupCols = tagKeys.length ? tagKeys : ['group'];
      return {
        columns: [...groupCols.map((k) => ({ key: k, label: k, type: 'string' as const })), ...aggregators.map((a) => ({ key: a, label: a, type: 'number' as const }))],
        rows: [...byGroup.values()],
      };
    }
    case 'logs': {
      if (source.groupBy.trim()) {
        const key = source.groupBy.trim();
        const values = await topLogValues({ range, query: source.query, key, limit: Math.min(Math.max(source.limit || 25, 1), 100) });
        return {
          columns: [
            { key, label: key, type: 'string' },
            { key: 'count', label: 'count', type: 'number' },
          ],
          rows: values.map((v) => ({ [key]: v.value, count: v.count })),
        };
      }
      const page = await searchLogPage({ range, query: source.query }, Math.min(Math.max(source.limit || LOG_ROWS, 1), 1000));
      return {
        columns: ['timestamp', 'status', 'service', 'host', 'message'].map((k) => ({ key: k, label: k, type: 'string' as const })),
        rows: page.logs.map((l) => ({ timestamp: new Date(l.timestamp).toISOString(), status: l.status, service: l.service, host: l.host, message: l.message })),
      };
    }
    case 'reference_table': {
      if (!source.table) return { columns: [], rows: [] };
      const page = await getReferenceTable(source.table, { limit: 1000, offset: 0 });
      const cols = page.table.columns;
      const rows = page.rows.map((r) => Object.fromEntries(cols.map((c, i) => [c, numeric(r[i])])));
      return { columns: cols.map((c) => ({ key: c, label: c, type: rows.length > 0 && rows.every((r) => typeof r[c] === 'number' || r[c] === null) ? 'number' : 'string' })), rows };
    }
  }
}

/** CSV cells that are numbers become numbers (so formulas and pivots can use them). */
function numeric(v: string | undefined): string | number | null {
  if (v === undefined || v === '') return null;
  const n = Number(v);
  return Number.isFinite(n) && /^-?\d/.test(v.trim()) ? n : v;
}
