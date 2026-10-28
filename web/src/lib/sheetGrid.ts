// Sheets: what a sheet shows, from its data source's table — calculated columns, then filters, then the
// pivot, then the sort. Pure functions over plain rows, shared by the editor and the tests.
import { compileFormula, type Cell, type Row } from './sheetFormula';

export type { Cell, Row };

export interface GridColumn {
  key: string;
  label: string;
  type: 'number' | 'string';
}

export interface GridTable {
  columns: GridColumn[];
  rows: Row[];
  /** Column → why some of its cells are empty (formula errors). */
  errors?: Record<string, string>;
}

export interface CalculatedColumn {
  id: string;
  name: string;
  formula: string;
}

export type FilterOp = '=' | '!=' | '>' | '>=' | '<' | '<=' | 'contains';
export const FILTER_OPS: FilterOp[] = ['=', '!=', '>', '>=', '<', '<=', 'contains'];

export interface SheetFilter {
  column: string;
  op: FilterOp;
  value: string;
}

export interface SheetSort {
  column: string;
  dir: 'asc' | 'desc';
}

export type PivotFn = 'sum' | 'avg' | 'min' | 'max' | 'count';
export const PIVOT_FNS: PivotFn[] = ['sum', 'avg', 'min', 'max', 'count'];

export interface SheetPivot {
  groupBy: string;
  aggregations: { column: string; fn: PivotFn }[];
}

export interface SheetView {
  calculated: CalculatedColumn[];
  filters: SheetFilter[];
  pivot: SheetPivot | null;
  sort: SheetSort | null;
}

const isNum = (v: Cell): v is number => typeof v === 'number' && Number.isFinite(v);

function columnType(rows: Row[], key: string): GridColumn['type'] {
  const values = rows.map((r) => r[key]).filter((v) => v !== null && v !== undefined && v !== '');
  return values.length > 0 && values.every(isNum) ? 'number' : 'string';
}

/** Adds the calculated columns in order (each may use the columns before it). */
export function applyCalculated(table: GridTable, calculated: CalculatedColumn[]): GridTable {
  const columns = [...table.columns];
  const rows = table.rows.map((r) => ({ ...r }));
  const errors: Record<string, string> = { ...table.errors };
  for (const c of calculated) {
    const name = c.name.trim() || 'Untitled';
    let evaluate: ((row: Row) => Cell) | null = null;
    try {
      evaluate = compileFormula(
        c.formula,
        columns.map((col) => col.key),
      );
    } catch (e) {
      errors[name] = (e as Error).message;
    }
    for (const r of rows) {
      if (!evaluate) {
        r[name] = null;
        continue;
      }
      try {
        r[name] = evaluate(r);
      } catch (e) {
        r[name] = null;
        errors[name] ??= (e as Error).message;
      }
    }
    columns.push({ key: name, label: name, type: columnType(rows, name) });
  }
  return { columns, rows, errors };
}

const text = (v: Cell | undefined) => (v === null || v === undefined ? '' : String(v)).toLowerCase();

function matches(v: Cell | undefined, f: SheetFilter): boolean {
  const want = f.value.trim();
  const n = Number(want);
  const numeric = want !== '' && Number.isFinite(n) && (isNum(v ?? null) || (typeof v === 'string' && v.trim() !== '' && Number.isFinite(Number(v))));
  if (f.op === 'contains') return text(v).includes(want.toLowerCase());
  if (numeric) {
    const x = Number(v);
    switch (f.op) {
      case '=':
        return x === n;
      case '!=':
        return x !== n;
      case '>':
        return x > n;
      case '>=':
        return x >= n;
      case '<':
        return x < n;
      case '<=':
        return x <= n;
    }
  }
  const c = text(v).localeCompare(want.toLowerCase());
  switch (f.op) {
    case '=':
      return c === 0;
    case '!=':
      return c !== 0;
    case '>':
      return c > 0;
    case '>=':
      return c >= 0;
    case '<':
      return c < 0;
    case '<=':
      return c <= 0;
  }
  return true;
}

/** Keeps the rows matching every filter (filters on columns the table lacks are ignored). */
export function applyFilters(table: GridTable, filters: SheetFilter[]): GridTable {
  const keys = new Set(table.columns.map((c) => c.key));
  const active = filters.filter((f) => keys.has(f.column));
  return { ...table, rows: table.rows.filter((r) => active.every((f) => matches(r[f.column], f))) };
}

/** Numbers numerically, text alphabetically; empty cells last. */
export function applySort(table: GridTable, sort: SheetSort | null): GridTable {
  if (!sort || !table.columns.some((c) => c.key === sort.column)) return table;
  const sign = sort.dir === 'asc' ? 1 : -1;
  const rows = [...table.rows].sort((a, b) => {
    const x = a[sort.column];
    const y = b[sort.column];
    const ex = x === null || x === undefined || x === '';
    const ey = y === null || y === undefined || y === '';
    if (ex || ey) return ex === ey ? 0 : ex ? 1 : -1;
    if (isNum(x) && isNum(y)) return sign * (x - y);
    return sign * String(x).localeCompare(String(y));
  });
  return { ...table, rows };
}

export const pivotKey = (a: { column: string; fn: PivotFn }) => `${a.fn}(${a.column})`;

/** One row per value of `groupBy` (in order of appearance), one column per aggregation. */
export function pivotTable(table: GridTable, pivot: SheetPivot): GridTable {
  if (!table.columns.some((c) => c.key === pivot.groupBy)) return table;
  const groups = new Map<string, { value: Cell; rows: Row[] }>();
  for (const r of table.rows) {
    const v = r[pivot.groupBy] ?? null;
    const k = v === null ? '' : String(v);
    if (!groups.has(k)) groups.set(k, { value: v, rows: [] });
    groups.get(k)!.rows.push(r);
  }
  const aggs = pivot.aggregations.filter((a) => table.columns.some((c) => c.key === a.column));
  const rows: Row[] = [...groups.values()].map((g) => {
    const out: Row = { [pivot.groupBy]: g.value };
    for (const a of aggs) {
      const present = g.rows.map((r) => r[a.column]).filter((v) => v !== null && v !== undefined && v !== '');
      const nums = present.map(Number).filter(Number.isFinite);
      let v: Cell = null;
      if (a.fn === 'count') v = present.length;
      else if (nums.length > 0) {
        if (a.fn === 'sum') v = nums.reduce((s, x) => s + x, 0);
        else if (a.fn === 'avg') v = nums.reduce((s, x) => s + x, 0) / nums.length;
        else if (a.fn === 'min') v = Math.min(...nums);
        else v = Math.max(...nums);
      }
      out[pivotKey(a)] = typeof v === 'number' ? Math.round(v * 1e6) / 1e6 : v;
    }
    return out;
  });
  const group = table.columns.find((c) => c.key === pivot.groupBy)!;
  return { columns: [group, ...aggs.map((a) => ({ key: pivotKey(a), label: pivotKey(a), type: 'number' as const }))], rows, errors: table.errors };
}

/** The table a sheet shows. */
export function computeSheet(source: GridTable, view: SheetView): GridTable {
  let t = applyCalculated(source, view.calculated);
  t = applyFilters(t, view.filters);
  if (view.pivot?.groupBy) t = pivotTable(t, view.pivot);
  return applySort(t, view.sort);
}
