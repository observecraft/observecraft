import { useState } from 'react';
import { Bookmark, Pencil, Plus, Trash2, X } from 'lucide-react';
import type { TemplateVariable, TemplateVarValues } from '../../../api/dashboards/types';
import { Select } from '../../../components/ui/Dropdown';
import { Modal } from '../../../components/ui/Modal';
import { useMetricCatalog, useTagValues } from '../queries';
import { visibleVariables } from '../tree';

interface SavedView {
  name: string;
  values: TemplateVarValues;
}

function readViews(id: string): SavedView[] {
  try {
    return JSON.parse(localStorage.getItem(`oc.dash.views.${id}`) ?? '[]') as SavedView[];
  } catch {
    return [];
  }
}

function writeViews(id: string, views: SavedView[]) {
  try {
    localStorage.setItem(`oc.dash.views.${id}`, JSON.stringify(views));
  } catch {
    /* storage unavailable: views last for this session only */
  }
}

function VarSelect({ tv, value, onChange }: { tv: TemplateVariable; value: string; onChange: (v: string) => void }) {
  const values = useTagValues(tv.prefix);
  return (
    <Select
      label={tv.name}
      value={value}
      options={[{ value: '*', label: '*' }, ...(values.data ?? []).map((v) => ({ value: v, label: v }))]}
      onChange={onChange}
      searchable
      triggerClassName={`dd-trigger tv-trigger${value !== '*' ? ' tv-active' : ''}`}
    />
  );
}

interface Props {
  dashboardId: string;
  variables: TemplateVariable[];
  values: TemplateVarValues;
  onValues: (v: TemplateVarValues) => void;
  canEdit: boolean;
  onEditVariables: () => void;
}

/** "Saved Views ▾  Filter by: [env ▾] [service ▾] …" row under the dashboard title. */
export function TemplateVarBar({ dashboardId, variables, values, onValues, canEdit, onEditVariables }: Props) {
  const [views, setViews] = useState(() => readViews(dashboardId));
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState('');
  const [showAll, setShowAll] = useState(false);
  const visible = visibleVariables(variables, values, showAll);

  const save = () => {
    const next = [...views.filter((v) => v.name !== name.trim()), { name: name.trim(), values }];
    setViews(next);
    writeViews(dashboardId, next);
    setNaming(false);
    setName('');
  };

  return (
    <div className="tv-bar">
      <Select
        value=""
        options={[
          { value: '', label: 'Saved Views' },
          { value: '__default', label: 'Default view' },
          ...views.map((v) => ({ value: v.name, label: v.name })),
          { value: '__save', label: 'Save current view…', icon: <Plus size={13} /> },
          ...(views.length ? [{ value: '__clear', label: 'Delete all saved views', icon: <Trash2 size={13} /> }] : []),
        ]}
        onChange={(v) => {
          if (v === '__save') setNaming(true);
          else if (v === '__default') onValues({});
          else if (v === '__clear') {
            setViews([]);
            writeViews(dashboardId, []);
          } else {
            const view = views.find((x) => x.name === v);
            if (view) onValues(view.values);
          }
        }}
        triggerClassName="dd-trigger tv-views"
        trigger={() => (
          <>
            <Bookmark size={14} /> Saved Views
          </>
        )}
      />
      {variables.length > 0 && <span className="tv-label">Filter by:</span>}
      {visible.shown.map((tv) => (
        <VarSelect key={tv.name} tv={tv} value={values[tv.name] ?? tv.default} onChange={(v) => onValues({ ...values, [tv.name]: v })} />
      ))}
      {(visible.hidden > 0 || showAll) && variables.length > 6 && (
        <button className="tv-more" onClick={() => setShowAll((s) => !s)} title={showAll ? 'Show fewer variables' : 'Show all variables'}>
          {showAll ? 'Less' : `+${visible.hidden}`}
        </button>
      )}
      {canEdit && (
        <button className="link-btn tv-edit" onClick={onEditVariables}>
          <Pencil size={13} /> {variables.length ? 'Edit' : 'Add template variables'}
        </button>
      )}

      {naming && (
        <Modal onClose={() => setNaming(false)} label="Save view" width={420} height="auto">
          <div className="small-modal">
            <h3>Save current view</h3>
            <input autoFocus placeholder="View name" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && name.trim() && save()} />
            <div className="small-modal-actions">
              <button className="btn" onClick={() => setNaming(false)}>
                Cancel
              </button>
              <button className="btn btn-primary" disabled={!name.trim()} onClick={save}>
                Save
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}

/** Add / remove / rename template variables. */
export function TemplateVarEditor({ variables, onSave, onClose }: { variables: TemplateVariable[]; onSave: (v: TemplateVariable[]) => void; onClose: () => void }) {
  const catalog = useMetricCatalog();
  const [rows, setRows] = useState<TemplateVariable[]>(variables);
  const keys = [...new Set((catalog.data ?? []).flatMap((m) => m.tags))].sort();
  return (
    <Modal onClose={onClose} label="Template variables" width={640} height="auto">
      <div className="small-modal">
        <h3>Template variables</h3>
        <p className="muted-text">
          Reference them in queries as <code>$name</code> inside the filter braces.
        </p>
        {rows.map((r, i) => (
          <div key={i} className="tv-edit-row">
            <input placeholder="name" value={r.name} onChange={(e) => setRows(rows.map((x, j) => (j === i ? { ...x, name: e.target.value.replace(/\W/g, '_') } : x)))} />
            <Select
              searchable
              value={r.prefix}
              options={keys.map((k) => ({ value: k, label: k }))}
              onChange={(prefix) => setRows(rows.map((x, j) => (j === i ? { ...x, prefix, name: x.name || prefix } : x)))}
            />
            <input placeholder="default (*)" value={r.default} onChange={(e) => setRows(rows.map((x, j) => (j === i ? { ...x, default: e.target.value || '*' } : x)))} />
            <button className="icon-plain" onClick={() => setRows(rows.filter((_, j) => j !== i))} title="Remove">
              <X size={15} />
            </button>
          </div>
        ))}
        <button className="link-btn" onClick={() => setRows([...rows, { name: 'env', prefix: 'env', default: '*' }])}>
          <Plus size={14} /> Add variable
        </button>
        <div className="small-modal-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={rows.some((r) => !r.name)} onClick={() => onSave(rows)}>
            Save
          </button>
        </div>
      </div>
    </Modal>
  );
}
