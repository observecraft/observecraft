import { BellPlus, BookOpen, ChevronDown, ChevronRight, Copy, Expand, EllipsisVertical, Info, Link2, Maximize2, Pencil, Trash2, Braces, Upload } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { useState, type ReactNode } from 'react';
import type { Widget, WidgetDefinition } from '../../../api/dashboards/types';
import { Menu } from '../../../components/ui/Dropdown';
import { useDashboardContext } from './context';
import { BarChartWidget, ChangeWidget, HeatmapWidget, PieWidget, QueryValueWidget, TableWidget, TimeseriesWidget, ToplistWidget } from './MetricWidgets';
import { ImageWidget, LogStreamWidget, MonitorSummaryWidget, NoteWidget } from './OtherWidgets';
import { IntegrationWidget, widgetSpan } from './IntegrationWidget';
import { MetricsInfoModal } from './MetricsInfoModal';
import { monitorQuery, widgetMetrics, widgetQueries } from './widgetQueries';
import { IntegrationLogo } from '../../integrations/IntegrationLogo';
import { AddToNotebookModal } from '../../notebooks/AddToNotebookModal';
import { cellFromWidget } from '../../notebooks/cells';

/** The URL parameter that opens a widget in full screen. */
export const FULLSCREEN_PARAM = 'fullscreen_widget';

/** Renders a widget's content (no frame). Groups are handled by the canvas. */
export function WidgetContent({ def }: { def: WidgetDefinition }) {
  switch (def.type) {
    case 'integration':
      return <IntegrationWidget def={def.original} />;
    case 'timeseries':
      return <TimeseriesWidget def={def} />;
    case 'query_value':
      return <QueryValueWidget def={def} />;
    case 'toplist':
      return <ToplistWidget def={def} />;
    case 'change':
      return <ChangeWidget def={def} />;
    case 'bar_chart':
      return <BarChartWidget def={def} />;
    case 'table':
      return <TableWidget def={def} />;
    case 'heatmap':
    case 'distribution':
      return <HeatmapWidget def={def} />;
    case 'pie':
    case 'treemap':
      return <PieWidget def={def} />;
    case 'note':
      return <NoteWidget def={def} />;
    case 'image':
      return <ImageWidget def={def} />;
    case 'monitor_summary':
      return <MonitorSummaryWidget def={def} />;
    case 'log_stream':
      return <LogStreamWidget def={def} />;
    case 'group':
      return null;
  }
}

const HEADERLESS = new Set(['note', 'image']);

export function WidgetMenu({ widget }: { widget: Widget }) {
  const ctx = useDashboardContext();
  const navigate = useNavigate();
  // Add to notebook: the widget as a notebook cell, its template variables resolved.
  const [notebookCell, setNotebookCell] = useState<ReturnType<typeof cellFromWidget>>(null);
  const [metricsInfo, setMetricsInfo] = useState(false);
  const asCell = () => cellFromWidget(widget.definition, (q) => ctx.resolve(q));
  const queries = widgetQueries(widget.definition);
  const monitor = monitorQuery(queries, ctx.resolve);
  const metrics = widgetMetrics(queries);
  const items = [
    { label: 'View in full screen', icon: <Expand size={14} />, onClick: () => ctx.onFullscreen(widget) },
    ...(monitor
      ? [
          {
            label: 'Create monitor',
            icon: <BellPlus size={14} />,
            onClick: () => navigate(`/monitors/create/metric?${new URLSearchParams({ query: monitor, name: widget.definition.title || '' })}`),
          },
        ]
      : []),
    ...(metrics.length ? [{ label: 'Metrics info', icon: <Info size={14} />, onClick: () => setMetricsInfo(true) }] : []),
    ...(ctx.canEdit
      ? [
          { label: 'Edit', icon: <Pencil size={14} />, onClick: () => ctx.onEdit(widget) },
          { label: 'Clone', icon: <Copy size={14} />, onClick: () => ctx.onClone(widget.id) },
        ]
      : []),
    { label: 'Copy JSON', icon: <Braces size={14} />, onClick: () => navigator.clipboard?.writeText(JSON.stringify(widget.definition, null, 2)) },
    ...(asCell() ? [{ label: 'Add to notebook', icon: <BookOpen size={14} />, onClick: () => setNotebookCell(asCell()) }] : []),
    ...(ctx.canEdit ? [{ label: 'Delete', icon: <Trash2 size={14} />, onClick: () => ctx.onDelete(widget.id) }] : []),
  ];
  return (
    <>
      <Menu trigger={<EllipsisVertical size={15} />} items={items} triggerClassName="wf-menu-btn" />
      {notebookCell && <AddToNotebookModal cells={[notebookCell]} newTitle={widget.definition.title || 'Notes from a dashboard'} onClose={() => setNotebookCell(null)} />}
      {metricsInfo && <MetricsInfoModal title={widget.definition.title || ''} metrics={metrics} onClose={() => setMetricsInfo(false)} />}
    </>
  );
}

/** This dashboard's address with the widget open in full screen. */
export function widgetLink(widget: Widget): string {
  const url = new URL(window.location.href);
  url.searchParams.set(FULLSCREEN_PARAM, widget.id);
  return url.toString();
}

/** Share a widget: a link that opens it in full screen, or its definition. */
export function WidgetShare({ widget, label }: { widget: Widget; label?: string }) {
  const items = [
    { label: 'Copy link', icon: <Link2 size={14} />, onClick: () => navigator.clipboard?.writeText(widgetLink(widget)) },
    { label: 'Copy JSON', icon: <Braces size={14} />, onClick: () => navigator.clipboard?.writeText(JSON.stringify(widget.definition, null, 2)) },
  ];
  return (
    <Menu
      trigger={
        <>
          <Upload size={label ? 14 : 15} />
          {label}
        </>
      }
      items={items}
      triggerClassName={label ? 'btn' : 'wf-menu-btn'}
    />
  );
}

/** Card around a widget: draggable title bar + hover menu. */
/** `dragClass` must differ per grid nesting level so a child's handle never drags its parent group. */
export function WidgetFrame({ widget, dragClass = 'drag-l0' }: { widget: Widget; dragClass?: string }) {
  const def = widget.definition;
  const headerless = HEADERLESS.has(def.type);
  const span = def.type === 'integration' ? widgetSpan(def.original) : undefined;
  const ctx = useDashboardContext();
  return (
    <div className={`wf wf-${def.type}${headerless ? ' wf-headerless' : ''}`}>
      <div className={`wf-head ${dragClass}`}>
        {!headerless && <span className="wf-title">{def.title || ''}</span>}
        {span && (
          <span className="wf-span" title={`This widget shows the last ${span.label}`}>
            {span.label}
          </span>
        )}
        <span className="wf-actions wf-nodrag">
          {!headerless && (
            <>
              <button type="button" className="wf-menu-btn" title="View in full screen" onClick={() => ctx.onFullscreen(widget)}>
                <Maximize2 size={14} />
              </button>
              <WidgetShare widget={widget} />
            </>
          )}
          <WidgetMenu widget={widget} />
        </span>
      </div>
      <div className="wf-body wf-nodrag-content">
        <WidgetContent def={def} />
      </div>
    </div>
  );
}

const GROUP_COLORS: Record<Extract<WidgetDefinition, { type: 'group' }>['color'], string> = {
  blue: 'var(--group-blue)',
  purple: 'var(--group-purple)',
  green: 'var(--group-green)',
  gray: 'var(--group-gray)',
  orange: 'var(--group-orange)',
};

/** Group header bar; children are rendered by the canvas in `children`. */
export function GroupFrame({ widget, children, dragClass = 'drag-l0' }: { widget: Widget; children: ReactNode; dragClass?: string }) {
  const def = widget.definition as Extract<WidgetDefinition, { type: 'group' }>;
  const [open, setOpen] = useState(true);
  if (def.showTitle === false)
    return (
      <div className="wg wg-untitled">
        <div className={`wg-handle ${dragClass}`}>
          <span className="wf-actions wf-nodrag wg-actions">
            <WidgetMenu widget={widget} />
          </span>
        </div>
        <div className="wg-body">{children}</div>
      </div>
    );
  return (
    <div className={`wg${def.compact ? ' wg-compact' : ''}${def.light ? ' wg-light' : ''}`}>
      <div className={`wg-head ${dragClass}`} style={{ background: def.light ? `var(--group-${def.color}-light)` : GROUP_COLORS[def.color] }}>
        <button className="wg-toggle wf-nodrag" onClick={() => setOpen((o) => !o)} title={open ? 'Collapse' : 'Expand'}>
          {open ? <ChevronDown size={18} /> : <ChevronRight size={18} />}
        </button>
        {def.logo && <IntegrationLogo logo={def.logo} name={def.title || def.logo} size={24} />}
        <span className="wg-title">{def.title}</span>
        <span className="wf-actions wf-nodrag wg-actions">
          <WidgetMenu widget={widget} />
        </span>
      </div>
      {open && <div className="wg-body">{children}</div>}
    </div>
  );
}
