import { useMemo } from 'react';
import GridLayout, { WidthProvider, type Layout } from 'react-grid-layout';
import 'react-grid-layout/css/styles.css';
import 'react-resizable/css/styles.css';
import type { LayoutType, Widget } from '../../../api/dashboards/types';
import { useDashboardContext } from '../widgets/context';
import { GroupFrame, WidgetFrame } from '../widgets/WidgetFrame';
import { groupHeight } from '../tree';

const Grid = WidthProvider(GridLayout);

/** Grid geometry per layout type. Free layout uses a 4× finer grid and allows overlap. */
export const GEOMETRY = {
  grid: { cols: 12, rowHeight: 40, margin: [12, 12] as [number, number] },
  free: { cols: 48, rowHeight: 10, margin: [6, 6] as [number, number] },
};

interface GridProps {
  widgets: Widget[];
  layoutType: Exclude<LayoutType, 'auto'>;
  editable: boolean;
  onLayout: (layouts: Layout[]) => void;
  nested?: boolean;
}

function WidgetGrid({ widgets, layoutType, editable, onLayout, nested = false }: GridProps) {
  const ctx = useDashboardContext();
  const g = GEOMETRY[layoutType];
  const dragClass = nested ? 'drag-l1' : 'drag-l0';
  const layout = useMemo<Layout[]>(
    () =>
      widgets.map((w) => {
        const group = w.definition.type === 'group';
        return {
          i: w.id,
          x: w.layout.x,
          y: w.layout.y,
          w: w.layout.w,
          h: group ? groupHeight(w.definition.type === 'group' ? w.definition.widgets : []) : w.layout.h,
          minW: layoutType === 'free' ? 4 : 2,
          minH: layoutType === 'free' ? 4 : 2,
          resizeHandles: group ? ['e'] : ['se'],
        };
      }),
    [widgets, layoutType],
  );
  return (
    <Grid
      className={`dash-grid${nested ? ' nested' : ''}`}
      layout={layout}
      cols={g.cols}
      rowHeight={g.rowHeight}
      margin={nested ? [10, 10] : g.margin}
      containerPadding={nested ? [10, 10] : [0, 0]}
      isDraggable={editable}
      isResizable={editable}
      draggableHandle={`.${dragClass}`}
      draggableCancel=".wf-nodrag"
      compactType={layoutType === 'free' ? null : 'vertical'}
      allowOverlap={layoutType === 'free'}
      onDragStop={onLayout}
      onResizeStop={onLayout}
    >
      {widgets.map((w) => (
        <div key={w.id} className="dash-cell">
          {w.definition.type === 'group' ? (
            <GroupFrame widget={w} dragClass={dragClass}>
              <WidgetGrid widgets={w.definition.widgets} layoutType="grid" editable={editable} nested onLayout={(l) => ctx.onGroupLayout(w.id, l)} />
            </GroupFrame>
          ) : (
            <WidgetFrame widget={w} dragClass={dragClass} />
          )}
        </div>
      ))}
    </Grid>
  );
}

interface Props {
  widgets: Widget[];
  layoutType: LayoutType;
  editable: boolean;
  onLayout: (layouts: Layout[]) => void;
}

/** Lays out a dashboard: snapped grid, free canvas, or automatic (timeboard) flow. */
export function DashboardCanvas({ widgets, layoutType, editable, onLayout }: Props) {
  if (layoutType === 'auto') {
    const ordered = [...widgets].sort((a, b) => a.layout.y - b.layout.y || a.layout.x - b.layout.x);
    return (
      <div className="dash-auto">
        {ordered.map((w) =>
          w.definition.type === 'group' ? (
            <div key={w.id} className="dash-auto-group">
              <GroupFrame widget={w}>
                <div className="dash-auto">
                  {w.definition.widgets.map((c) => (
                    <div key={c.id} className="dash-auto-cell">
                      <WidgetFrame widget={c} />
                    </div>
                  ))}
                </div>
              </GroupFrame>
            </div>
          ) : (
            <div key={w.id} className="dash-auto-cell">
              <WidgetFrame widget={w} />
            </div>
          ),
        )}
      </div>
    );
  }
  return <WidgetGrid widgets={widgets} layoutType={layoutType} editable={editable} onLayout={onLayout} />;
}
