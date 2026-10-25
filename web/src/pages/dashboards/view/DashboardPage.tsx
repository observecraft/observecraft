import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import type { Layout } from 'react-grid-layout';
import { Check, ChevronDown, Copy, Download, LayoutDashboard, Plus, Settings, Share, Siren, Star, Trash2, Upload, Variable, X } from 'lucide-react';
import type { Dashboard, TemplateVarValues, Widget, WidgetDefinition, WidgetType } from '../../../api/dashboards/types';
import { TimePicker } from '../../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Menu } from '../../../components/ui/Dropdown';
import { Modal } from '../../../components/ui/Modal';
import { resolveQuery } from '../../../lib/metricQuery';
import { snapRange } from '../../../lib/time';
import { IntegrationLogo } from '../../integrations/IntegrationLogo';
import { AddWidgetsDrawer } from '../editor/AddWidgetsDrawer';
import { WidgetEditor } from '../editor/WidgetEditor';
import { useCloneDashboard, useDashboard, useDeleteDashboards, useUpdateDashboard } from '../queries';
import { DEFAULT_SIZE, applyLayouts, bottom, cloneWidget, defaultDefinition, newWidgetId, removeWidget, updateWidget } from '../tree';
import { DashboardContext, type DashboardContextValue } from '../widgets/context';
import { GroupFrame, WidgetFrame } from '../widgets/WidgetFrame';
import { DashboardCanvas } from './DashboardCanvas';
import { TemplateVarBar, TemplateVarEditor } from './TemplateVarBar';
import '../dashboards.css';

const VAR_PREFIX = 'tpl_var_';

export function DashboardPage() {
  const { id } = useParams();
  return (
    <TimeProvider initial="1h">
      <DashboardView id={id!} />
    </TimeProvider>
  );
}

function DashboardView({ id }: { id: string }) {
  const q = useDashboard(id);
  if (q.isPending) return <Loading label="Loading dashboard…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <DashboardBody dashboard={q.data} />;
}

function download(name: string, text: string) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
}

function DashboardBody({ dashboard: d }: { dashboard: Dashboard }) {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const update = useUpdateDashboard(d.id);
  const clone = useCloneDashboard();
  const del = useDeleteDashboards();
  const { range: live } = useTime();
  const { from, to } = snapRange(live);
  const range = useMemo(() => ({ from, to }), [from, to]);
  const canEdit = d.kind === 'custom' && d.deletedAt === null;

  const [drawer, setDrawer] = useState(false);
  const [editing, setEditing] = useState<{ widget: Widget; isNew: boolean } | null>(null);
  const [fullscreen, setFullscreen] = useState<Widget | null>(null);
  const [tvEditor, setTvEditor] = useState(false);
  const [importing, setImporting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState(d.title);
  const [toast, setToast] = useState<{ text: string; undo?: Widget[] } | null>(null);
  const [copied, setCopied] = useState(false);
  const toastTimer = useRef<number>();

  // Template variable values live in the URL (?tpl_var_env=prod) so views are shareable.
  const values = useMemo<TemplateVarValues>(() => {
    const out: TemplateVarValues = {};
    for (const tv of d.templateVariables) out[tv.name] = params.get(VAR_PREFIX + tv.name) ?? tv.default;
    return out;
  }, [d.templateVariables, params]);
  const setValues = (v: TemplateVarValues) =>
    setParams(
      (p) => {
        const next = new URLSearchParams(p);
        for (const tv of d.templateVariables) {
          const val = v[tv.name] ?? tv.default;
          if (val === tv.default) next.delete(VAR_PREFIX + tv.name);
          else next.set(VAR_PREFIX + tv.name, val);
        }
        return next;
      },
      { replace: true },
    );

  const showToast = useCallback((text: string, undo?: Widget[]) => {
    window.clearTimeout(toastTimer.current);
    setToast({ text, undo });
    toastTimer.current = window.setTimeout(() => setToast(null), 6000);
  }, []);
  useEffect(() => () => window.clearTimeout(toastTimer.current), []);

  const { mutate } = update; // stable across renders
  const saveWidgets = useCallback((widgets: Widget[]) => mutate({ widgets }), [mutate]);

  const ctx = useMemo<DashboardContextValue>(
    () => ({
      range,
      resolve: (query) => resolveQuery(query, d.templateVariables, values),
      resolveSearch: (query) =>
        [query, ...d.templateVariables.filter((tv) => values[tv.name] && values[tv.name] !== '*').map((tv) => `${tv.prefix}:${values[tv.name]}`)].filter(Boolean).join(' '),
      canEdit,
      onEdit: (w) => setEditing({ widget: w, isNew: false }),
      onClone: (wid) => saveWidgets(cloneWidget(d.widgets, wid)),
      onDelete: (wid) => {
        showToast('Widget deleted', d.widgets);
        saveWidgets(removeWidget(d.widgets, wid));
      },
      onFullscreen: setFullscreen,
      onGroupLayout: (gid, layouts) =>
        saveWidgets(updateWidget(d.widgets, gid, (g) => (g.definition.type === 'group' ? { ...g, definition: { ...g.definition, widgets: applyLayouts(g.definition.widgets, layouts) } } : g))),
    }),
    [range, d.widgets, d.templateVariables, values, canEdit, saveWidgets, showToast],
  );

  const onLayout = (layouts: Layout[]) => saveWidgets(applyLayouts(d.widgets, layouts));

  const pick = (type: WidgetType) => {
    const size = DEFAULT_SIZE[type];
    const scale = d.layoutType === 'free' ? 4 : 1;
    setEditing({ widget: { id: newWidgetId(), definition: defaultDefinition(type), layout: { x: 0, y: Infinity, w: size.w * scale, h: size.h * scale } }, isNew: true });
  };

  const onEditorSave = (def: WidgetDefinition, groupId: string | null) => {
    if (!editing) return;
    if (!editing.isNew) {
      saveWidgets(updateWidget(d.widgets, editing.widget.id, (w) => ({ ...w, definition: def })));
    } else if (groupId) {
      saveWidgets(
        updateWidget(d.widgets, groupId, (g) =>
          g.definition.type === 'group' ? { ...g, definition: { ...g.definition, widgets: [...g.definition.widgets, { ...editing.widget, definition: def, layout: { ...editing.widget.layout, y: bottom(g.definition.widgets) } }] } } : g,
        ),
      );
    } else {
      saveWidgets([...d.widgets, { ...editing.widget, definition: def, layout: { ...editing.widget.layout, y: bottom(d.widgets) } }]);
    }
    setEditing(null);
  };

  const doClone = () => clone.mutate(d.id, { onSuccess: (c) => navigate(`/dashboards/${c.id}`) });
  const commitTitle = () => {
    setRenaming(false);
    if (title.trim() && title.trim() !== d.title) update.mutate({ title: title.trim() });
    else setTitle(d.title);
  };

  const groups = d.widgets.filter((w) => w.definition.type === 'group').map((w) => ({ id: w.id, title: w.definition.title || 'Group' }));

  return (
    <DashboardContext.Provider value={ctx}>
      <div className="dash-page">
        <header className="dash-header">
          <div className="dash-title-row">
            <button className={`icon-plain dash-star${d.starred ? ' on' : ''}`} title={d.starred ? 'Unstar' : 'Star'} onClick={() => update.mutate({ starred: !d.starred })}>
              <Star size={18} fill={d.starred ? 'currentColor' : 'none'} />
            </button>
            <span className="dash-icon">
              {d.logo === 'tile:monitors' ? <Siren size={18} color="var(--status-warn)" /> : d.logo ? <IntegrationLogo logo={d.logo} name={d.title} size={20} /> : <LayoutDashboard size={18} />}
            </span>
            {renaming ? (
              <input className="dash-title-input" autoFocus value={title} onChange={(e) => setTitle(e.target.value)} onBlur={commitTitle} onKeyDown={(e) => e.key === 'Enter' && commitTitle()} />
            ) : (
              <Menu
                triggerClassName="dash-title"
                align="left"
                trigger={
                  <>
                    {d.title} <ChevronDown size={16} />
                  </>
                }
                items={[
                  { label: 'Rename', onClick: () => setRenaming(true), disabled: !canEdit },
                  { label: d.description ? `About: ${d.description}` : 'No description', onClick: () => undefined, disabled: true },
                  { label: 'Back to all dashboards', onClick: () => navigate('/dashboards') },
                ]}
              />
            )}
            {update.isPending && <span className="dash-saving muted-text">Saving…</span>}
            {d.kind === 'integration' && <span className="tag-chip dash-readonly">Read-only · clone to edit</span>}
            <div className="dash-actions">
              <button
                className="link-btn dash-action"
                onClick={() => {
                  navigator.clipboard?.writeText(window.location.href);
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1500);
                }}
              >
                {copied ? <Check size={15} /> : <Share size={15} />} {copied ? 'Link copied' : 'Share'}
              </button>
              <Menu
                triggerClassName="link-btn dash-action"
                trigger={
                  <>
                    <Settings size={15} /> Configure
                  </>
                }
                items={[
                  { label: 'Template variables', icon: <Variable size={14} />, onClick: () => setTvEditor(true), disabled: !canEdit },
                  { label: 'Export dashboard JSON', icon: <Download size={14} />, onClick: () => download(`${d.title}.json`, JSON.stringify({ title: d.title, layoutType: d.layoutType, templateVariables: d.templateVariables, widgets: d.widgets }, null, 2)) },
                  { label: 'Import dashboard JSON', icon: <Upload size={14} />, onClick: () => setImporting(true), disabled: !canEdit },
                  { label: 'Clone dashboard', icon: <Copy size={14} />, onClick: doClone },
                  { label: 'Delete dashboard', icon: <Trash2 size={14} />, onClick: () => setConfirmDelete(true), disabled: !canEdit },
                ]}
              />
              {canEdit ? (
                <>
                  <button className="btn" onClick={doClone} disabled={clone.isPending}>
                    <Copy size={14} /> Clone
                  </button>
                  <button className="btn btn-primary" onClick={() => setDrawer((o) => !o)}>
                    {drawer ? <X size={14} /> : <Plus size={14} />} Add Widgets
                  </button>
                </>
              ) : (
                <button className="btn btn-primary" onClick={doClone} disabled={clone.isPending}>
                  <Copy size={14} /> {clone.isPending ? 'Cloning…' : 'Clone'}
                </button>
              )}
            </div>
          </div>
          <div className="dash-filter-row">
            <TemplateVarBar dashboardId={d.id} variables={d.templateVariables} values={values} onValues={setValues} canEdit={canEdit} onEditVariables={() => setTvEditor(true)} />
            <TimePicker />
          </div>
        </header>

        <div className={`dash-scroll${drawer ? ' with-drawer' : ''}`}>
          {d.widgets.length === 0 ? (
            <div className="dash-empty">
              <LayoutDashboard size={40} />
              <h3>This dashboard is empty</h3>
              <p>Add widgets to start graphing metrics, logs and monitors.</p>
              {canEdit && (
                <button className="btn btn-primary" onClick={() => setDrawer(true)}>
                  <Plus size={14} /> Add Widgets
                </button>
              )}
            </div>
          ) : (
            <DashboardCanvas widgets={d.widgets} layoutType={d.layoutType} editable={canEdit} onLayout={onLayout} />
          )}
        </div>

        {drawer && canEdit && <AddWidgetsDrawer onPick={pick} onClose={() => setDrawer(false)} />}

        {editing && <WidgetEditor widget={editing.widget} isNew={editing.isNew} groups={groups} onSave={onEditorSave} onClose={() => setEditing(null)} />}

        {fullscreen && (
          <Modal onClose={() => setFullscreen(null)} label="Widget" width="92vw">
            <div className="dash-fullscreen">
              {fullscreen.definition.type === 'group' ? (
                <GroupFrame widget={fullscreen}>
                  <DashboardCanvas widgets={fullscreen.definition.widgets} layoutType="grid" editable={false} onLayout={() => undefined} />
                </GroupFrame>
              ) : (
                <WidgetFrame widget={fullscreen} />
              )}
            </div>
          </Modal>
        )}

        {tvEditor && (
          <TemplateVarEditor
            variables={d.templateVariables}
            onClose={() => setTvEditor(false)}
            onSave={(templateVariables) => {
              update.mutate({ templateVariables });
              setTvEditor(false);
            }}
          />
        )}

        {importing && <ImportModal onClose={() => setImporting(false)} onImport={(patch) => (update.mutate(patch), setImporting(false))} />}

        {confirmDelete && (
          <Modal onClose={() => setConfirmDelete(false)} label="Delete dashboard" width={460} height="auto">
            <div className="small-modal">
              <h3>Delete “{d.title}”?</h3>
              <p className="muted-text">It moves to Recently Deleted and can be restored from there.</p>
              <div className="small-modal-actions">
                <button className="btn" onClick={() => setConfirmDelete(false)}>
                  Cancel
                </button>
                <button className="btn btn-danger" onClick={() => del.mutate([d.id], { onSuccess: () => navigate('/dashboards') })}>
                  Delete
                </button>
              </div>
            </div>
          </Modal>
        )}

        {toast && (
          <div className="toast" role="status">
            {toast.text}
            {toast.undo && (
              <button
                className="link-btn"
                onClick={() => {
                  saveWidgets(toast.undo!);
                  setToast(null);
                }}
              >
                Undo
              </button>
            )}
          </div>
        )}
      </div>
    </DashboardContext.Provider>
  );
}

function ImportModal({ onClose, onImport }: { onClose: () => void; onImport: (patch: Partial<Pick<Dashboard, 'widgets' | 'templateVariables' | 'layoutType'>>) => void }) {
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);
  const submit = () => {
    try {
      const json = JSON.parse(text) as Partial<Dashboard>;
      if (!Array.isArray(json.widgets)) throw new Error('JSON must contain a "widgets" array.');
      onImport({ widgets: json.widgets, templateVariables: Array.isArray(json.templateVariables) ? json.templateVariables : [], ...(json.layoutType ? { layoutType: json.layoutType } : {}) });
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Invalid JSON');
    }
  };
  return (
    <Modal onClose={onClose} label="Import dashboard JSON" width={720} height="auto">
      <div className="small-modal">
        <h3>Import dashboard JSON</h3>
        <p className="muted-text">Replaces this dashboard&apos;s widgets and template variables.</p>
        <textarea className="mono import-area" rows={14} value={text} onChange={(e) => (setText(e.target.value), setError(null))} placeholder='{"widgets": [...], "templateVariables": [...]}' />
        {error && <p className="state-error">{error}</p>}
        <div className="small-modal-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!text.trim()} onClick={submit}>
            Import
          </button>
        </div>
      </div>
    </Modal>
  );
}
