import { useMemo, useState } from 'react';
import { ArrowDown, ArrowUp, Check, Copy, Pencil, Plus, SlidersHorizontal, Trash2, X } from 'lucide-react';
import type { MetricRequest, TimeAggregator, Widget, WidgetDefinition } from '../../api/dashboards/types';
import { CELL_TYPES, type CellDefinition, type CellType, type NotebookCell, type NotebookTime } from '../../api/notebooks/types';
import { RELATIVE_RANGES } from '../../components/shell/TimeContext';
import { Menu, Select } from '../../components/ui/Dropdown';
import { snapRange } from '../../lib/time';
import type { TimeRange } from '../../api/types';
import { QueryBuilder } from '../dashboards/editor/QueryBuilder';
import { WidgetEditor } from '../dashboards/editor/WidgetEditor';
import { DashboardContext, type DashboardContextValue } from '../dashboards/widgets/context';
import { Markdown } from '../dashboards/widgets/markdown';
import { WidgetContent } from '../dashboards/widgets/WidgetFrame';
import { CELL_LABEL } from './cells';

const TIME_AGGS: TimeAggregator[] = ['avg', 'sum', 'min', 'max', 'last'];
const MAX_REQUESTS = 4;

type QueryDefinition = Exclude<CellDefinition, { type: 'markdown' }>;

/** The cell's time range: its own relative range ending with the notebook's, or the notebook's. */
export function cellRange(notebook: TimeRange, time: NotebookTime | null | undefined): TimeRange {
  const own = time && RELATIVE_RANGES.find((r) => r.key === time);
  return snapRange(own ? { from: notebook.to - own.ms, to: notebook.to } : notebook);
}

/** Widgets render inside a DashboardContext: a read-only one with the cell's range. */
function CellContext({ range, children }: { range: TimeRange; children: React.ReactNode }) {
  const ctx = useMemo<DashboardContextValue>(
    () => ({ range, resolve: (q) => q, resolveSearch: (q) => q, canEdit: false, onEdit: () => {}, onClone: () => {}, onDelete: () => {}, onFullscreen: () => {}, onGroupLayout: () => {} }),
    [range],
  );
  return <DashboardContext.Provider value={ctx}>{children}</DashboardContext.Provider>;
}

/** "+" between cells: adds a cell of the chosen type at `index`. */
export function InsertBar({ onInsert, prominent }: { onInsert: (type: CellType) => void; prominent?: boolean }) {
  const items = CELL_TYPES.map((t) => ({ label: CELL_LABEL[t], onClick: () => onInsert(t) }));
  if (prominent)
    return (
      <div className="nb-add-row">
        {CELL_TYPES.map((t) => (
          <button key={t} className="btn btn-sm" onClick={() => onInsert(t)}>
            <Plus size={14} /> {CELL_LABEL[t]}
          </button>
        ))}
      </div>
    );
  return (
    <div className="nb-insert">
      <Menu align="left" triggerClassName="nb-insert-btn" trigger={<Plus size={14} />} items={items} />
    </div>
  );
}

interface CellProps {
  cell: NotebookCell;
  index: number;
  count: number;
  range: TimeRange;
  canEdit: boolean;
  editing: boolean;
  onEditing: (editing: boolean) => void;
  onChange: (cell: NotebookCell) => void;
  onMove: (direction: -1 | 1) => void;
  onDuplicate: () => void;
  onDelete: () => void;
}

/** One cell: its content, and on hover the toolbar to edit, move, duplicate or delete it. */
export function CellView({ cell, index, count, range, canEdit, editing, onEditing, onChange, onMove, onDuplicate, onDelete }: CellProps) {
  const def = cell.definition;
  const timeOptions = [{ value: '', label: 'Notebook time' }, ...RELATIVE_RANGES.map((r) => ({ value: r.key, label: r.label }))];
  return (
    <div className={`nb-cell nb-cell-${def.type}${editing ? ' editing' : ''}`}>
      {canEdit && (
        <div className="nb-cell-tools">
          <span className="nb-cell-kind">{CELL_LABEL[def.type]}</span>
          {def.type !== 'markdown' && (
            <Select
              value={cell.time ?? ''}
              options={timeOptions}
              onChange={(t) => onChange({ ...cell, time: (t || null) as NotebookTime | null })}
              triggerClassName="dd-trigger nb-cell-time"
              title="Time range of this cell"
            />
          )}
          <button className="icon-plain" title={editing ? 'Done' : 'Edit'} onClick={() => onEditing(!editing)}>
            {editing ? <Check size={15} /> : <Pencil size={15} />}
          </button>
          <button className="icon-plain" title="Move up" disabled={index === 0} onClick={() => onMove(-1)}>
            <ArrowUp size={15} />
          </button>
          <button className="icon-plain" title="Move down" disabled={index === count - 1} onClick={() => onMove(1)}>
            <ArrowDown size={15} />
          </button>
          <button className="icon-plain" title="Duplicate" onClick={onDuplicate}>
            <Copy size={15} />
          </button>
          <button className="icon-plain" title="Delete" onClick={onDelete}>
            <Trash2 size={15} />
          </button>
        </div>
      )}
      {def.type === 'markdown' ? (
        <MarkdownCell text={def.text} editing={editing} onEdit={canEdit ? () => onEditing(true) : undefined} onChange={(text) => onChange({ ...cell, definition: { type: 'markdown', text } })} />
      ) : (
        <CellContext range={cellRange(range, cell.time)}>
          <QueryCell def={def} id={cell.id} editing={editing} time={cell.time} onChange={(definition) => onChange({ ...cell, definition })} />
        </CellContext>
      )}
    </div>
  );
}

function MarkdownCell({ text, editing, onEdit, onChange }: { text: string; editing: boolean; onEdit?: () => void; onChange: (text: string) => void }) {
  if (!editing)
    return (
      <div className="nb-markdown" onDoubleClick={onEdit}>
        {text.trim() ? <Markdown source={text} /> : <p className="muted-text">Empty text cell. {onEdit ? 'Double-click to write.' : ''}</p>}
      </div>
    );
  return (
    <div className="nb-markdown-edit">
      <textarea
        className="mono"
        autoFocus
        value={text}
        placeholder="Write Markdown: # headings, **bold**, `code`, - lists, [links](https://…)"
        onChange={(e) => onChange(e.target.value)}
        rows={Math.max(6, text.split('\n').length + 1)}
      />
      <div className="nb-markdown nb-markdown-preview">{text.trim() ? <Markdown source={text} /> : <p className="muted-text">Preview</p>}</div>
    </div>
  );
}

const HEIGHT: Record<QueryDefinition['type'], number> = { timeseries: 260, query_value: 150, toplist: 260, log_stream: 320 };

function QueryCell({ def, id, editing, time, onChange }: { def: QueryDefinition; id: string; editing: boolean; time?: NotebookTime | null; onChange: (def: QueryDefinition) => void }) {
  const [advanced, setAdvanced] = useState(false);
  const widget: Widget = { id, definition: def, layout: { x: 0, y: 0, w: 12, h: 6 } };
  return (
    <>
      <div className={`wf wf-${def.type} nb-widget`} style={{ height: HEIGHT[def.type] }}>
        <div className="wf-head">
          <span className="wf-title">{def.title || CELL_LABEL[def.type]}</span>
          {time && <span className="nb-time-badge">{RELATIVE_RANGES.find((r) => r.key === time)?.short}</span>}
        </div>
        <div className="wf-body">
          <WidgetContent def={def} />
        </div>
      </div>
      {editing && (
        <div className="nb-query-edit">
          <label className="we-field">
            <span>Title</span>
            <input value={def.title ?? ''} placeholder={CELL_LABEL[def.type]} onChange={(e) => onChange({ ...def, title: e.target.value })} />
          </label>
          <QueryFields def={def} onChange={onChange} />
          <button className="link-btn" onClick={() => setAdvanced(true)}>
            <SlidersHorizontal size={14} /> All options
          </button>
        </div>
      )}
      {advanced && (
        <WidgetEditor
          widget={widget}
          isNew={false}
          groups={[]}
          onClose={() => setAdvanced(false)}
          onSave={(next: WidgetDefinition) => {
            if (next.type === def.type) onChange(next as QueryDefinition);
            setAdvanced(false);
          }}
        />
      )}
    </>
  );
}

/** The compact query editor of a cell: metric queries (with their aggregation) or a log search. */
function QueryFields({ def, onChange }: { def: QueryDefinition; onChange: (def: QueryDefinition) => void }) {
  const aggregator = (value: TimeAggregator, set: (a: TimeAggregator) => void) => (
    <label className="we-field">
      <span>Aggregate over time</span>
      <Select value={value} options={TIME_AGGS.map((a) => ({ value: a, label: a === 'last' ? 'last value' : a }))} onChange={set} />
    </label>
  );
  if (def.type === 'log_stream')
    return (
      <div className="nb-query-row">
        <label className="we-field nb-grow">
          <span>Log search</span>
          <input className="mono" value={def.query} placeholder="service:order-service status:error" onChange={(e) => onChange({ ...def, query: e.target.value })} />
        </label>
        <label className="we-field">
          <span>Rows</span>
          <input type="number" min={1} max={100} value={def.limit} onChange={(e) => onChange({ ...def, limit: Math.min(100, Math.max(1, Number(e.target.value) || 1)) })} />
        </label>
      </div>
    );
  if (def.type === 'timeseries') {
    const setRequests = (requests: MetricRequest[]) => onChange({ ...def, requests });
    return (
      <div className="nb-requests">
        {def.requests.map((r, i) => (
          <div key={i} className="we-request">
            <span className="we-letter">{String.fromCharCode(97 + i)}</span>
            <QueryBuilder value={r.query} onChange={(query) => setRequests(def.requests.map((x, j) => (j === i ? { ...x, query } : x)))} />
            {def.requests.length > 1 && (
              <button className="icon-plain" title="Remove query" onClick={() => setRequests(def.requests.filter((_, j) => j !== i))}>
                <X size={15} />
              </button>
            )}
          </div>
        ))}
        {def.requests.length < MAX_REQUESTS && (
          <button className="link-btn" onClick={() => setRequests([...def.requests, { query: def.requests[def.requests.length - 1]?.query ?? 'avg:system.load.1{*}' }])}>
            <Plus size={14} /> Add query
          </button>
        )}
      </div>
    );
  }
  return (
    <div className="nb-query-row">
      <div className="we-request nb-grow">
        <QueryBuilder value={def.request.query} onChange={(query) => onChange({ ...def, request: { ...def.request, query } })} />
      </div>
      {aggregator(def.aggregator, (a) => onChange({ ...def, aggregator: a }))}
      {def.type === 'toplist' && (
        <label className="we-field">
          <span>Limit</span>
          <input type="number" min={1} max={100} value={def.limit} onChange={(e) => onChange({ ...def, limit: Math.min(100, Math.max(1, Number(e.target.value) || 1)) })} />
        </label>
      )}
    </div>
  );
}
