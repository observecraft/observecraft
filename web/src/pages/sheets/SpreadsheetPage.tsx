import { useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { ArrowDown, ArrowUp, ChevronLeft, EllipsisVertical, FileSpreadsheet, Filter, Plus, RefreshCw, Rows3, Sigma, X } from 'lucide-react';
import type { TimeAggregator } from '../../api/dashboards/types';
import { listReferenceTables } from '../../api/referenceTables/client';
import { SHEET_TIMES, newSheet, type Sheet, type SheetSource, type Spreadsheet } from '../../api/sheets/types';
import { RELATIVE_RANGES } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { FILTER_OPS, PIVOT_FNS, computeSheet, pivotKey, type Cell, type FilterOp, type GridTable, type PivotFn } from '../../lib/sheetGrid';
import { compileFormula } from '../../lib/sheetFormula';
import { useDeleteSpreadsheets, useSheetData, useSpreadsheet, useUpdateSpreadsheet } from './queries';
import '../dashboards/dashboards.css';
import './sheets.css';

const AGGREGATORS: TimeAggregator[] = ['avg', 'sum', 'min', 'max', 'last'];
const SOURCE_LABELS: Record<SheetSource['type'], string> = { none: 'No data source', metrics: 'Metrics', logs: 'Logs', reference_table: 'Reference table' };

function defaultSource(type: SheetSource['type']): SheetSource {
  switch (type) {
    case 'metrics':
      return { type, query: 'avg:trace.latency.p95{env:prod} by {service}', aggregators: ['avg', 'max'] };
    case 'logs':
      return { type, query: 'status:error', groupBy: 'service', limit: 25 };
    case 'reference_table':
      return { type, table: '' };
    default:
      return { type: 'none' };
  }
}

export function SpreadsheetPage() {
  const { id } = useParams();
  const q = useSpreadsheet(id!);
  if (q.isPending) return <Loading label="Loading spreadsheet…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <SpreadsheetEditor key={q.data.id} spreadsheet={q.data} />;
}

function SpreadsheetEditor({ spreadsheet }: { spreadsheet: Spreadsheet }) {
  const navigate = useNavigate();
  const update = useUpdateSpreadsheet();
  const del = useDeleteSpreadsheets();
  const [name, setName] = useState(spreadsheet.name);
  const [team, setTeam] = useState(spreadsheet.team ?? '');
  const [sheets, setSheets] = useState<Sheet[]>(spreadsheet.definition.sheets.length ? spreadsheet.definition.sheets : [newSheet(1)]);
  const [active, setActive] = useState(0);
  const [dirty, setDirty] = useState(false);
  const sheet = sheets[Math.min(active, sheets.length - 1)];

  const change = (patch: Partial<Sheet>) => {
    setSheets((ss) => ss.map((s, i) => (i === Math.min(active, ss.length - 1) ? { ...s, ...patch } : s)));
    setDirty(true);
  };
  const save = () => update.mutate({ id: spreadsheet.id, patch: { name, team, definition: { sheets } } }, { onSuccess: () => setDirty(false) });

  // Ctrl/Cmd+S saves.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 's') {
        e.preventDefault();
        save();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  return (
    <div className="dl-page ss-page">
      <header className="product-header ss-header">
        <Link to="/sheets" className="link-btn" title="All spreadsheets">
          <ChevronLeft size={16} /> Sheets
        </Link>
        <FileSpreadsheet size={18} className="muted" />
        <input className="ss-name" aria-label="Spreadsheet name" value={name} onChange={(e) => (setName(e.target.value), setDirty(true))} />
        <label className="ss-team">
          Team
          <input placeholder="none" value={team} onChange={(e) => (setTeam(e.target.value), setDirty(true))} />
        </label>
        <div className="dl-header-actions">
          {update.error && <span className="form-error">{update.error.message}</span>}
          <span className="muted-text">{update.isPending ? 'Saving…' : dirty ? 'Unsaved changes' : `Saved · by ${spreadsheet.author?.name ?? 'unknown'}`}</span>
          <button className="btn btn-primary" disabled={!dirty || update.isPending || !name.trim()} onClick={save}>
            Save
          </button>
          <Menu
            triggerClassName="icon-plain"
            trigger={<EllipsisVertical size={16} />}
            items={[{ label: 'Delete spreadsheet', onClick: () => del.mutate([spreadsheet.id], { onSuccess: () => navigate('/sheets') }) }]}
          />
        </div>
      </header>

      <div className="ss-tabs" role="tablist">
        {sheets.map((s, i) => (
          <div key={s.id} className={`ss-tab${i === active ? ' active' : ''}`} role="tab" aria-selected={i === active} onClick={() => setActive(i)}>
            {i === active ? <input aria-label="Sheet name" value={s.name} onChange={(e) => change({ name: e.target.value })} size={Math.max(6, s.name.length)} /> : s.name}
            {sheets.length > 1 && i === active && (
              <button
                className="icon-plain"
                title="Delete sheet"
                onClick={(e) => {
                  e.stopPropagation();
                  setSheets((ss) => ss.filter((_, j) => j !== i));
                  setActive(Math.max(0, i - 1));
                  setDirty(true);
                }}
              >
                <X size={12} />
              </button>
            )}
          </div>
        ))}
        {sheets.length < 20 && (
          <button
            className="icon-plain ss-add-tab"
            title="Add sheet"
            onClick={() => {
              setSheets((ss) => [...ss, newSheet(ss.length + 1)]);
              setActive(sheets.length);
              setDirty(true);
            }}
          >
            <Plus size={14} />
          </button>
        )}
      </div>

      <SheetView key={sheet.id} sheet={sheet} onChange={change} />
    </div>
  );
}

function SourceBar({ sheet, onChange, onRefresh }: { sheet: Sheet; onChange: (p: Partial<Sheet>) => void; onRefresh: () => void }) {
  const s = sheet.source;
  const tables = useQuery({ queryKey: ['reference-tables'], queryFn: listReferenceTables, enabled: s.type === 'reference_table' });
  const [draft, setDraft] = useState(s.type === 'metrics' || s.type === 'logs' ? s.query : '');
  const commit = () => (s.type === 'metrics' || s.type === 'logs') && draft !== s.query && onChange({ source: { ...s, query: draft } });
  const pick = (type: SheetSource['type']) => {
    const next = defaultSource(type);
    onChange({ source: next });
    setDraft(next.type === 'metrics' || next.type === 'logs' ? next.query : '');
  };
  return (
    <div className="ss-source">
      <select aria-label="Data source" value={s.type} onChange={(e) => pick(e.target.value as SheetSource['type'])}>
        {(Object.keys(SOURCE_LABELS) as SheetSource['type'][]).map((t) => (
          <option key={t} value={t}>
            {SOURCE_LABELS[t]}
          </option>
        ))}
      </select>
      {(s.type === 'metrics' || s.type === 'logs') && (
        <input
          className="ss-query"
          aria-label="Query"
          placeholder={s.type === 'metrics' ? 'avg:metric{tag:value} by {tag}' : 'service:billing status:error'}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === 'Enter' && commit()}
          spellCheck={false}
        />
      )}
      {s.type === 'metrics' && (
        <span className="ss-aggs" title="One column per time aggregation">
          {AGGREGATORS.map((a) => (
            <label key={a}>
              <input
                type="checkbox"
                className="checkbox"
                checked={s.aggregators.includes(a)}
                onChange={() => onChange({ source: { ...s, aggregators: s.aggregators.includes(a) ? s.aggregators.filter((x) => x !== a) : [...s.aggregators, a] } })}
              />{' '}
              {a}
            </label>
          ))}
        </span>
      )}
      {s.type === 'logs' && (
        <>
          <label className="ss-inline">
            group by
            <input placeholder="(rows) e.g. service" value={s.groupBy} onChange={(e) => onChange({ source: { ...s, groupBy: e.target.value } })} />
          </label>
          <label className="ss-inline">
            limit
            <input type="number" min={1} max={1000} value={s.limit} onChange={(e) => onChange({ source: { ...s, limit: Number(e.target.value) || 25 } })} />
          </label>
        </>
      )}
      {s.type === 'reference_table' && (
        <select aria-label="Reference table" value={s.table} onChange={(e) => onChange({ source: { ...s, table: e.target.value } })}>
          <option value="">Choose a table</option>
          {(tables.data ?? []).map((t) => (
            <option key={t.name} value={t.name}>
              {t.name}
            </option>
          ))}
        </select>
      )}
      {(s.type === 'metrics' || s.type === 'logs') && (
        <select aria-label="Time frame" value={sheet.time} onChange={(e) => onChange({ time: e.target.value as Sheet['time'] })}>
          {SHEET_TIMES.map((t) => (
            <option key={t} value={t}>
              {RELATIVE_RANGES.find((r) => r.key === t)?.label ?? t}
            </option>
          ))}
        </select>
      )}
      {s.type !== 'none' && (
        <button className="btn btn-sm" onClick={onRefresh} title="Run the query again">
          <RefreshCw size={13} /> Refresh
        </button>
      )}
    </div>
  );
}

function SheetView({ sheet, onChange }: { sheet: Sheet; onChange: (p: Partial<Sheet>) => void }) {
  const [version, setVersion] = useState(0);
  const [panel, setPanel] = useState<'columns' | 'filters' | 'pivot' | null>(sheet.calculated.length ? 'columns' : null);
  const data = useSheetData(sheet.source, sheet.time, version);
  const source: GridTable = data.data ?? { columns: [], rows: [] };
  const table = useMemo(() => computeSheet(source, sheet), [source, sheet]);
  // Columns available to formulas, filters and the pivot (before pivoting).
  const available = useMemo(() => computeSheet(source, { ...sheet, pivot: null, filters: [], sort: null }).columns.map((c) => c.key), [source, sheet]);
  const sortBy = (column: string) => onChange({ sort: sheet.sort?.column === column && sheet.sort.dir === 'desc' ? { column, dir: 'asc' } : { column, dir: 'desc' } });
  const toggle = (p: typeof panel) => setPanel((x) => (x === p ? null : p));

  return (
    <div className="ss-body">
      <SourceBar sheet={sheet} onChange={onChange} onRefresh={() => setVersion((v) => v + 1)} />
      <div className="ss-tools">
        <button className={`btn btn-sm${panel === 'columns' ? ' active' : ''}`} onClick={() => toggle('columns')}>
          <Sigma size={13} /> Calculated columns{sheet.calculated.length ? ` (${sheet.calculated.length})` : ''}
        </button>
        <button className={`btn btn-sm${panel === 'filters' ? ' active' : ''}`} onClick={() => toggle('filters')}>
          <Filter size={13} /> Filters{sheet.filters.length ? ` (${sheet.filters.length})` : ''}
        </button>
        <button className={`btn btn-sm${panel === 'pivot' ? ' active' : ''}`} onClick={() => toggle('pivot')}>
          <Rows3 size={13} /> Pivot{sheet.pivot ? ' (on)' : ''}
        </button>
        <span className="muted-text ss-meta">
          {table.rows.length} row{table.rows.length === 1 ? '' : 's'}
          {data.isFetching ? ' · loading…' : ''}
        </span>
      </div>

      {panel === 'columns' && <CalculatedPanel sheet={sheet} available={available} errors={table.errors ?? {}} onChange={onChange} />}
      {panel === 'filters' && <FiltersPanel sheet={sheet} available={available} onChange={onChange} />}
      {panel === 'pivot' && <PivotPanel sheet={sheet} available={available} onChange={onChange} />}

      <div className="ss-grid-wrap">
        {data.isError ? (
          <ErrorState error={data.error} onRetry={() => data.refetch()} />
        ) : sheet.source.type === 'none' && table.columns.length === 0 ? (
          <div className="dash-empty">
            <FileSpreadsheet size={36} />
            <h3>Choose a data source</h3>
            <p>Back this sheet with a metric query, a log search or a reference table, then add calculated columns, filters and a pivot.</p>
          </div>
        ) : data.isPending ? (
          <Loading label="Running the query…" />
        ) : (
          <table className="ss-grid">
            <thead>
              <tr>
                <th className="ss-rownum" />
                {table.columns.map((c) => (
                  <th key={c.key} className={c.type === 'number' ? 'num' : undefined} onClick={() => sortBy(c.key)} title={table.errors?.[c.key] ?? 'Sort'}>
                    {c.label}
                    {sheet.sort?.column === c.key && (sheet.sort.dir === 'desc' ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
                    {table.errors?.[c.key] && <span className="ss-err-dot" />}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {table.rows.slice(0, 2000).map((r, i) => (
                <tr key={i}>
                  <td className="ss-rownum">{i + 1}</td>
                  {table.columns.map((c) => (
                    <td key={c.key} className={c.type === 'number' ? 'num' : undefined}>
                      {show(r[c.key])}
                    </td>
                  ))}
                </tr>
              ))}
              {table.rows.length === 0 && (
                <tr>
                  <td className="muted-text ss-none" colSpan={table.columns.length + 1}>
                    No rows{sheet.filters.length ? ' match the filters' : ''}.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

function show(v: Cell | undefined): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'number') return Number.isInteger(v) ? v.toLocaleString('en-US') : v.toLocaleString('en-US', { maximumFractionDigits: 4 });
  if (typeof v === 'boolean') return v ? 'TRUE' : 'FALSE';
  return v;
}

function CalculatedPanel({ sheet, available, errors, onChange }: { sheet: Sheet; available: string[]; errors: Record<string, string>; onChange: (p: Partial<Sheet>) => void }) {
  const set = (i: number, patch: Partial<Sheet['calculated'][number]>) => onChange({ calculated: sheet.calculated.map((c, j) => (j === i ? { ...c, ...patch } : c)) });
  return (
    <div className="ss-panel">
      <p className="muted-text ss-help">
        Formulas use the columns by name (<code>[name with spaces]</code> in brackets), numbers, "text", + − * / % ^, comparisons, &amp; and IF, ROUND, CONCAT, ABS, MIN, MAX, AND, OR, NOT.
      </p>
      {sheet.calculated.map((c, i) => {
        // Each formula may use the source's columns and the calculated columns before it.
        const before = available.filter((k) => !sheet.calculated.slice(i).some((x) => x.name === k));
        let syntax: string | null = null;
        try {
          compileFormula(c.formula, before);
        } catch (e) {
          syntax = (e as Error).message;
        }
        const error = syntax ?? errors[c.name];
        return (
          <div key={c.id} className="ss-panel-row">
            <input className="ss-col-name" aria-label="Column name" value={c.name} onChange={(e) => set(i, { name: e.target.value })} />
            <span className="muted">=</span>
            <input
              className="ss-formula"
              aria-label="Formula"
              value={c.formula}
              onChange={(e) => set(i, { formula: e.target.value })}
              spellCheck={false}
              placeholder="ROUND(errors / requests * 100, 2)"
            />
            <button className="icon-plain" title="Remove column" onClick={() => onChange({ calculated: sheet.calculated.filter((_, j) => j !== i) })}>
              <X size={14} />
            </button>
            {error && <span className="form-error ss-row-error">{error}</span>}
          </div>
        );
      })}
      <button
        className="btn btn-sm"
        onClick={() =>
          onChange({
            calculated: [
              ...sheet.calculated,
              { id: `c${Date.now().toString(36)}`, name: `column ${sheet.calculated.length + 1}`, formula: available.find((k) => k !== 'group') ? `${available[available.length - 1]} * 1` : '1' },
            ],
          })
        }
      >
        <Plus size={13} /> Add calculated column
      </button>
    </div>
  );
}

function FiltersPanel({ sheet, available, onChange }: { sheet: Sheet; available: string[]; onChange: (p: Partial<Sheet>) => void }) {
  const set = (i: number, patch: Partial<Sheet['filters'][number]>) => onChange({ filters: sheet.filters.map((f, j) => (j === i ? { ...f, ...patch } : f)) });
  return (
    <div className="ss-panel">
      {sheet.filters.map((f, i) => (
        <div key={i} className="ss-panel-row">
          <select aria-label="Column" value={f.column} onChange={(e) => set(i, { column: e.target.value })}>
            {[...new Set([f.column, ...available])].map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
          <select aria-label="Operator" value={f.op} onChange={(e) => set(i, { op: e.target.value as FilterOp })}>
            {FILTER_OPS.map((o) => (
              <option key={o} value={o}>
                {o}
              </option>
            ))}
          </select>
          <input aria-label="Value" value={f.value} onChange={(e) => set(i, { value: e.target.value })} />
          <button className="icon-plain" title="Remove filter" onClick={() => onChange({ filters: sheet.filters.filter((_, j) => j !== i) })}>
            <X size={14} />
          </button>
        </div>
      ))}
      <button className="btn btn-sm" disabled={available.length === 0} onClick={() => onChange({ filters: [...sheet.filters, { column: available[0], op: '=', value: '' }] })}>
        <Plus size={13} /> Add filter
      </button>
    </div>
  );
}

function PivotPanel({ sheet, available, onChange }: { sheet: Sheet; available: string[]; onChange: (p: Partial<Sheet>) => void }) {
  const p = sheet.pivot;
  if (!p)
    return (
      <div className="ss-panel">
        <button
          className="btn btn-sm"
          disabled={available.length === 0}
          onClick={() => onChange({ pivot: { groupBy: available[0], aggregations: available.slice(1, 2).map((column) => ({ column, fn: 'sum' as PivotFn })) }, sort: null })}
        >
          <Rows3 size={13} /> Pivot this sheet
        </button>
      </div>
    );
  const setAgg = (i: number, patch: Partial<(typeof p.aggregations)[number]>) => onChange({ pivot: { ...p, aggregations: p.aggregations.map((a, j) => (j === i ? { ...a, ...patch } : a)) } });
  return (
    <div className="ss-panel">
      <div className="ss-panel-row">
        <span>Group by</span>
        <select aria-label="Group by" value={p.groupBy} onChange={(e) => onChange({ pivot: { ...p, groupBy: e.target.value } })}>
          {available.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
        <button className="link-btn" onClick={() => onChange({ pivot: null, sort: null })}>
          Remove pivot
        </button>
      </div>
      {p.aggregations.map((a, i) => (
        <div key={i} className="ss-panel-row">
          <select aria-label="Aggregation" value={a.fn} onChange={(e) => setAgg(i, { fn: e.target.value as PivotFn })}>
            {PIVOT_FNS.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
          </select>
          <span>of</span>
          <select aria-label="Column" value={a.column} onChange={(e) => setAgg(i, { column: e.target.value })}>
            {available.map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
          <span className="muted-text">→ {pivotKey(a)}</span>
          <button className="icon-plain" title="Remove aggregation" onClick={() => onChange({ pivot: { ...p, aggregations: p.aggregations.filter((_, j) => j !== i) } })}>
            <X size={14} />
          </button>
        </div>
      ))}
      <button className="btn btn-sm" onClick={() => onChange({ pivot: { ...p, aggregations: [...p.aggregations, { column: available.find((k) => k !== p.groupBy) ?? p.groupBy, fn: 'sum' }] } })}>
        <Plus size={13} /> Add aggregation
      </button>
    </div>
  );
}
