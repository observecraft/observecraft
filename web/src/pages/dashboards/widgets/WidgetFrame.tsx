import { ChevronDown, ChevronRight, Copy, Expand, EllipsisVertical, Pencil, Trash2, Braces } from 'lucide-react';
import { useState, type ReactNode } from 'react';
import type { Widget, WidgetDefinition } from '../../../api/dashboards/types';
import { Menu } from '../../../components/ui/Dropdown';
import { useDashboardContext } from './context';
import { BarChartWidget, ChangeWidget, HeatmapWidget, PieWidget, QueryValueWidget, TableWidget, TimeseriesWidget, ToplistWidget } from './MetricWidgets';
import { ImageWidget, LogStreamWidget, MonitorSummaryWidget, NoteWidget } from './OtherWidgets';
import { IntegrationWidget } from './IntegrationWidget';
import { IntegrationLogo } from '../../integrations/IntegrationLogo';

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
  const items = [
    { label: 'View in full screen', icon: <Expand size={14} />, onClick: () => ctx.onFullscreen(widget) },
    ...(ctx.canEdit
      ? [
          { label: 'Edit', icon: <Pencil size={14} />, onClick: () => ctx.onEdit(widget) },
          { label: 'Clone', icon: <Copy size={14} />, onClick: () => ctx.onClone(widget.id) },
        ]
      : []),
    { label: 'Copy JSON', icon: <Braces size={14} />, onClick: () => navigator.clipboard?.writeText(JSON.stringify(widget.definition, null, 2)) },
    ...(ctx.canEdit ? [{ label: 'Delete', icon: <Trash2 size={14} />, onClick: () => ctx.onDelete(widget.id) }] : []),
  ];
  return <Menu trigger={<EllipsisVertical size={15} />} items={items} triggerClassName="wf-menu-btn" />;
}

/** Card around a widget: draggable title bar + hover menu. */
/** `dragClass` must differ per grid nesting level so a child's handle never drags its parent group. */
export function WidgetFrame({ widget, dragClass = 'drag-l0' }: { widget: Widget; dragClass?: string }) {
  const def = widget.definition;
  const headerless = HEADERLESS.has(def.type);
  return (
    <div className={`wf wf-${def.type}${headerless ? ' wf-headerless' : ''}`}>
      <div className={`wf-head ${dragClass}`}>
        {!headerless && <span className="wf-title">{def.title || ''}</span>}
        <span className="wf-actions wf-nodrag">
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
  return (
    <div className={`wg${def.compact ? ' wg-compact' : ''}`}>
      <div className={`wg-head ${dragClass}`} style={{ background: GROUP_COLORS[def.color] }}>
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
