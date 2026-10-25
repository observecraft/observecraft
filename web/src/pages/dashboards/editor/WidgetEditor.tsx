import { useState, type ReactNode } from 'react';
import { Plus, Trash2, X } from 'lucide-react';
import type { ConditionalFormat, MetricRequest, TimeAggregator, Widget, WidgetDefinition } from '../../../api/dashboards/types';
import { Select } from '../../../components/ui/Dropdown';
import { Modal } from '../../../components/ui/Modal';
import { GroupFrame, WidgetFrame } from '../widgets/WidgetFrame';
import { QueryBuilder } from './QueryBuilder';
import { WIDGET_LABEL } from './WidgetTiles';

const TIME_AGGS: TimeAggregator[] = ['avg', 'sum', 'min', 'max', 'last'];
const aggOptions = TIME_AGGS.map((a) => ({ value: a, label: a === 'last' ? 'last value' : a }));

function Field({ label, children, wide }: { label: string; children: ReactNode; wide?: boolean }) {
  return (
    <label className={`we-field${wide ? ' wide' : ''}`}>
      <span>{label}</span>
      {children}
    </label>
  );
}

function Requests({ requests, onChange, multiple, aliases }: { requests: MetricRequest[]; onChange: (r: MetricRequest[]) => void; multiple: boolean; aliases: boolean }) {
  return (
    <div className="we-requests">
      {requests.map((r, i) => (
        <div key={i} className="we-request">
          <span className="we-letter">{String.fromCharCode(97 + i)}</span>
          <QueryBuilder value={r.query} onChange={(query) => onChange(requests.map((x, j) => (j === i ? { ...x, query } : x)))} />
          {aliases && (
            <input className="we-alias" placeholder="alias" value={r.alias ?? ''} onChange={(e) => onChange(requests.map((x, j) => (j === i ? { ...x, alias: e.target.value || undefined } : x)))} />
          )}
          {multiple && requests.length > 1 && (
            <button className="icon-plain" title="Remove query" onClick={() => onChange(requests.filter((_, j) => j !== i))}>
              <Trash2 size={15} />
            </button>
          )}
        </div>
      ))}
      {multiple && requests.length < 4 && (
        <button className="link-btn" onClick={() => onChange([...requests, { query: requests[requests.length - 1]?.query ?? 'avg:system.cpu.user{*}' }])}>
          <Plus size={14} /> Add query
        </button>
      )}
    </div>
  );
}

function Formats({ formats, onChange }: { formats: ConditionalFormat[]; onChange: (f: ConditionalFormat[]) => void }) {
  return (
    <div className="we-formats">
      {formats.map((f, i) => (
        <div key={i} className="we-format">
          <span>If value is</span>
          <Select value={f.comparator} options={['>', '>=', '<', '<='].map((c) => ({ value: c as ConditionalFormat['comparator'], label: c }))} onChange={(c) => onChange(formats.map((x, j) => (j === i ? { ...x, comparator: c } : x)))} />
          <input type="number" value={f.value} onChange={(e) => onChange(formats.map((x, j) => (j === i ? { ...x, value: Number(e.target.value) } : x)))} />
          <span>show</span>
          <Select
            value={f.palette}
            options={(['green', 'yellow', 'red', 'blue', 'gray'] as const).map((p) => ({ value: p, label: p, icon: <span className={`we-swatch sw-${p}`} /> }))}
            onChange={(p) => onChange(formats.map((x, j) => (j === i ? { ...x, palette: p } : x)))}
          />
          <button className="icon-plain" onClick={() => onChange(formats.filter((_, j) => j !== i))} title="Remove rule">
            <Trash2 size={14} />
          </button>
        </div>
      ))}
      <button className="link-btn" onClick={() => onChange([...formats, { comparator: '>', value: 0, palette: 'red' }])}>
        <Plus size={14} /> Add conditional format
      </button>
    </div>
  );
}

interface Props {
  widget: Widget;
  isNew: boolean;
  /** Groups the new widget can be placed in. */
  groups: { id: string; title: string }[];
  onSave: (def: WidgetDefinition, groupId: string | null) => void;
  onClose: () => void;
}

export function WidgetEditor({ widget, isNew, groups, onSave, onClose }: Props) {
  const [def, setDef] = useState<WidgetDefinition>(widget.definition);
  const [target, setTarget] = useState<string>('');
  // Each branch below narrows `def`; patches keep the discriminant intact.
  const patch = (p: Partial<WidgetDefinition>) => setDef((d) => ({ ...d, ...p }) as WidgetDefinition);
  const preview: Widget = { ...widget, definition: def };

  return (
    <Modal onClose={onClose} label="Widget editor" width={1100}>
      <header className="we-header">
        <h3>
          {isNew ? 'New' : 'Edit'} {WIDGET_LABEL[def.type]}
        </h3>
        <button className="icon-plain" onClick={onClose} title="Close">
          <X size={20} />
        </button>
      </header>
      <div className="we-body">
        <div className="we-preview">
          {def.type === 'group' ? (
            <GroupFrame widget={preview}>
              <div className="we-group-preview">Widgets placed in this group appear here.</div>
            </GroupFrame>
          ) : (
            <WidgetFrame widget={preview} />
          )}
        </div>

        <div className="we-form">
          {def.type !== 'note' && def.type !== 'image' && (
            <Field label="Title" wide>
              <input value={def.title ?? ''} placeholder="Widget title" onChange={(e) => patch({ title: e.target.value })} />
            </Field>
          )}

          {def.type === 'timeseries' && (
            <>
              <h4>Graph your data</h4>
              <Requests requests={def.requests} onChange={(requests) => patch({ requests })} multiple aliases />
              <h4>Display</h4>
              <div className="we-row">
                <Field label="Display as">
                  <Select value={def.display} options={[{ value: 'line', label: 'Lines' }, { value: 'area', label: 'Areas' }, { value: 'bars', label: 'Bars' }]} onChange={(display) => patch({ display })} />
                </Field>
                <Field label="Stacked">
                  <input type="checkbox" className="checkbox" checked={!!def.stacked} onChange={(e) => patch({ stacked: e.target.checked })} />
                </Field>
                <Field label="Y max">
                  <input type="number" value={def.yMax ?? ''} placeholder="auto" onChange={(e) => patch({ yMax: e.target.value === '' ? undefined : Number(e.target.value) })} />
                </Field>
              </div>
            </>
          )}

          {def.type === 'table' && (
            <>
              <h4>Columns (one query each; rows come from the first query&apos;s “by” tags)</h4>
              <Requests requests={def.requests} onChange={(requests) => patch({ requests })} multiple aliases />
              <div className="we-row">
                <Field label="Aggregate over time">
                  <Select value={def.aggregator} options={aggOptions} onChange={(aggregator) => patch({ aggregator })} />
                </Field>
                <Field label="Rows">
                  <input type="number" min={1} max={200} value={def.limit} onChange={(e) => patch({ limit: Number(e.target.value) })} />
                </Field>
                <Field label="Search box">
                  <input type="checkbox" className="checkbox" checked={!!def.searchable} onChange={(e) => patch({ searchable: e.target.checked })} />
                </Field>
              </div>
            </>
          )}

          {(def.type === 'query_value' || def.type === 'toplist' || def.type === 'change' || def.type === 'bar_chart' || def.type === 'pie' || def.type === 'treemap' || def.type === 'heatmap' || def.type === 'distribution') && (
            <>
              <h4>Graph your data</h4>
              <Requests requests={[def.request]} onChange={([request]) => patch({ request })} multiple={false} aliases={false} />
              <div className="we-row">
                {'aggregator' in def && (
                  <Field label="Aggregate over time">
                    <Select value={def.aggregator} options={aggOptions} onChange={(aggregator) => patch({ aggregator } as Partial<WidgetDefinition>)} />
                  </Field>
                )}
                {'limit' in def && (
                  <Field label="Limit">
                    <input type="number" min={1} max={100} value={def.limit} onChange={(e) => patch({ limit: Number(e.target.value) } as Partial<WidgetDefinition>)} />
                  </Field>
                )}
                {def.type === 'query_value' && (
                  <>
                    <Field label="Decimals">
                      <input type="number" min={0} max={4} value={def.precision} onChange={(e) => patch({ precision: Number(e.target.value) })} />
                    </Field>
                    <Field label="Unit">
                      <input value={def.unit ?? ''} placeholder="e.g. ms, %" onChange={(e) => patch({ unit: e.target.value || undefined })} />
                    </Field>
                    <Field label="Fill background">
                      <input type="checkbox" className="checkbox" checked={!!def.fill} onChange={(e) => patch({ fill: e.target.checked })} />
                    </Field>
                  </>
                )}
              </div>
              {def.type === 'query_value' && (
                <>
                  <h4>Conditional formatting (first match wins)</h4>
                  <Formats formats={def.formats} onChange={(formats) => patch({ formats })} />
                </>
              )}
            </>
          )}

          {def.type === 'note' && (
            <>
              <Field label="Content (Markdown)" wide>
                <textarea rows={8} value={def.content} onChange={(e) => patch({ content: e.target.value })} className="mono" />
              </Field>
              <div className="we-row">
                <Field label="Background">
                  <Select value={def.background} options={(['white', 'yellow', 'blue', 'purple', 'green', 'red', 'gray'] as const).map((b) => ({ value: b, label: b, icon: <span className={`we-swatch note-sw-${b}`} /> }))} onChange={(background) => patch({ background })} />
                </Field>
                <Field label="Align">
                  <Select value={def.align} options={[{ value: 'left', label: 'Left' }, { value: 'center', label: 'Center' }]} onChange={(align) => patch({ align })} />
                </Field>
                <Field label="Text size">
                  <Select value={def.fontSize} options={(['sm', 'md', 'lg', 'xl'] as const).map((f) => ({ value: f, label: f.toUpperCase() }))} onChange={(fontSize) => patch({ fontSize })} />
                </Field>
                <Field label="Pointer">
                  <input type="checkbox" className="checkbox" checked={!!def.pointer} onChange={(e) => patch({ pointer: e.target.checked })} />
                </Field>
              </div>
            </>
          )}

          {def.type === 'image' && (
            <div className="we-row">
              <Field label="Image URL (https or data:)" wide>
                <input value={def.src ?? ''} placeholder="https://…" onChange={(e) => patch({ src: e.target.value || undefined })} />
              </Field>
              <Field label="…or logo key">
                <input value={def.logo ?? ''} placeholder="kubernetes" onChange={(e) => patch({ logo: e.target.value || undefined })} />
              </Field>
              <Field label="Background (CSS)">
                <input value={def.background ?? ''} placeholder="#1fb6ff" onChange={(e) => patch({ background: e.target.value || undefined })} />
              </Field>
            </div>
          )}

          {def.type === 'group' && (
            <Field label="Header color">
              <Select value={def.color} options={(['blue', 'purple', 'green', 'orange', 'gray'] as const).map((c) => ({ value: c, label: c }))} onChange={(color) => patch({ color })} />
            </Field>
          )}

          {(def.type === 'monitor_summary' || def.type === 'log_stream') && (
            <div className="we-row">
              <Field label={def.type === 'monitor_summary' ? 'Monitor search (e.g. team:billing env:prod)' : 'Log search (e.g. service:billing status:error)'} wide>
                <input className="mono" value={def.query} onChange={(e) => patch({ query: e.target.value })} />
              </Field>
              {def.type === 'log_stream' && (
                <Field label="Rows">
                  <input type="number" min={1} max={100} value={def.limit} onChange={(e) => patch({ limit: Number(e.target.value) })} />
                </Field>
              )}
            </div>
          )}

          {isNew && def.type !== 'group' && groups.length > 0 && (
            <Field label="Place in">
              <Select value={target} options={[{ value: '', label: 'Dashboard (bottom)' }, ...groups.map((g) => ({ value: g.id, label: `Group: ${g.title}` }))]} onChange={setTarget} />
            </Field>
          )}
        </div>
      </div>
      <footer className="we-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn btn-primary" onClick={() => onSave(def, target || null)}>
          {isNew ? 'Add widget' : 'Save'}
        </button>
      </footer>
    </Modal>
  );
}
