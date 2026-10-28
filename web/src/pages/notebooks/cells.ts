// Pure helpers over notebook cells: new cells, reordering, and conversion from and to dashboard widgets.
import type { Widget, WidgetDefinition } from '../../api/dashboards/types';
import type { CellDefinition, CellType, NotebookCell } from '../../api/notebooks/types';
import { newCellId } from '../../api/notebooks/templates';
import { DEFAULT_SIZE, defaultDefinition, newWidgetId } from '../dashboards/tree';

export const CELL_LABEL: Record<CellType, string> = {
  markdown: 'Text',
  timeseries: 'Timeseries',
  query_value: 'Query Value',
  toplist: 'Top List',
  log_stream: 'Log Stream',
};

/** A cell's starting definition. */
export function newCellDefinition(type: CellType): CellDefinition {
  if (type === 'markdown') return { type, text: '' };
  if (type === 'timeseries') return { type, title: '', display: 'line', requests: [{ query: 'avg:trace.latency.p95{env:prod} by {service}' }] };
  if (type === 'query_value') return { type, title: '', request: { query: 'avg:trace.error_rate{env:prod}' }, aggregator: 'avg', precision: 2, formats: [] };
  if (type === 'toplist') return { type, title: '', request: { query: 'sum:trace.errors{env:prod} by {service}' }, aggregator: 'sum', limit: 10 };
  return { ...(defaultDefinition('log_stream') as Extract<CellDefinition, { type: 'log_stream' }>), limit: 20 };
}

/** `cells` with `cell` inserted at `index`. */
export function insertCell(cells: NotebookCell[], index: number, cell: NotebookCell): NotebookCell[] {
  return [...cells.slice(0, index), cell, ...cells.slice(index)];
}

/** `cells` with the cell `id` moved one place up (-1) or down (1); unchanged at either end. */
export function moveCell(cells: NotebookCell[], id: string, direction: -1 | 1): NotebookCell[] {
  const from = cells.findIndex((c) => c.id === id);
  const to = from + direction;
  if (from < 0 || to < 0 || to >= cells.length) return cells;
  const out = [...cells];
  [out[from], out[to]] = [out[to], out[from]];
  return out;
}

/** `cells` with a copy of the cell `id` right after it. */
export function duplicateCell(cells: NotebookCell[], id: string): NotebookCell[] {
  const i = cells.findIndex((c) => c.id === id);
  if (i < 0) return cells;
  return insertCell(cells, i + 1, { ...structuredClone(cells[i]), id: newCellId() });
}

/** A dashboard widget as a notebook cell (queries already resolved), or null for widgets a notebook can't hold. */
export function cellFromWidget(def: WidgetDefinition, resolve: (query: string) => string = (q) => q): NotebookCell | null {
  const definition: CellDefinition | null =
    def.type === 'timeseries'
      ? { ...def, requests: def.requests.map((r) => ({ ...r, query: resolve(r.query) })) }
      : def.type === 'query_value' || def.type === 'toplist'
        ? { ...def, request: { ...def.request, query: resolve(def.request.query) } }
        : def.type === 'log_stream'
          ? { ...def }
          : def.type === 'note'
            ? { type: 'markdown', text: def.content }
            : null;
  return definition && { id: newCellId(), definition: structuredClone(definition) };
}

/** The cells as dashboard widgets, stacked full width in notebook order (Export to dashboard). */
export function cellsToWidgets(cells: NotebookCell[]): Widget[] {
  let y = 0;
  return cells.map((cell) => {
    const def = cell.definition;
    const definition: WidgetDefinition = def.type === 'markdown' ? { type: 'note', content: def.text, background: 'white', align: 'left', fontSize: 'md' } : def;
    const h = def.type === 'markdown' ? 4 : DEFAULT_SIZE[def.type].h;
    const widget: Widget = { id: newWidgetId(), definition: structuredClone(definition), layout: { x: 0, y, w: 12, h } };
    y += h;
    return widget;
  });
}
