import { forwardRef, memo, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import type { CloudMapEdge, CloudMapOverlay } from '../../api/cloudMap/client';
import type { Cloud } from '../../api/infrastructure/types';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { token } from '../../theme/chartTheme';
import { TYPE_LABELS } from './catalogModel';
import { TypeIcon } from './CatalogPanel';
import {
  BLOCK,
  BLOCK_HEIGHT,
  boxFaces,
  fitView,
  HEADER,
  levelZ,
  onScreen,
  PAD,
  paintOrder,
  planeTransform,
  project,
  screenBounds,
  serviceAnchors,
  SLAB,
  zoomAt,
  type DiagramLayout,
  type PlacedBlock,
  type PlacedGroup,
  type ViewMode,
  type Viewport,
} from './cloudMapLayout';
import { blockColor, DEFAULT_PALETTE, faces, groupColor, type Palette } from './cloudMapStyle';
import { CLOUD_LABEL, CloudIcon } from './hostIcons';

/** Pixels per grid unit at zoom 1. */
const UNIT = 26;
/** Zoom from which blocks get their icon, and from which they get a name. */
const ICON_ZOOM = 0.55;
const LABEL_ZOOM = 1.15;
const MAX_LABELS = 400;
const CLOUDS = new Set(Object.keys(CLOUD_LABEL));

export interface DiagramHandle {
  zoomIn: () => void;
  zoomOut: () => void;
  fit: () => void;
  /** The whole diagram as a standalone SVG document. */
  toSvg: () => string;
}

interface Props {
  layout: DiagramLayout;
  mode: ViewMode;
  overlay: CloudMapOverlay;
  edges: CloudMapEdge[];
  selectedId: string | null;
  onSelect: (b: PlacedBlock | null) => void;
  /** A label for an empty group value ("No region", "Cluster-wide"). */
  groupLabel: (key: string, value: string) => string;
}

function readPalette(): Palette {
  const t = (name: string, fallback: string) => token(name) || fallback;
  const d = DEFAULT_PALETTE;
  return {
    brand: t('--brand', d.brand),
    blue: t('--group-blue', d.blue),
    purple: t('--group-purple', d.purple),
    orange: t('--group-orange', d.orange),
    green: t('--group-green', d.green),
    gray: t('--group-gray', d.gray),
    ok: t('--status-ok', d.ok),
    warn: t('--status-warn', d.warn),
    error: t('--status-error', d.error),
    nodata: t('--status-nodata', d.nodata),
    text: t('--text', d.text),
    muted: t('--text-muted', d.muted),
    border: t('--border', d.border),
    bg: t('--bg', d.bg),
  };
}

/** The mark drawn on a group's label strip. */
function groupMark(k: string, value: string, size: number): ReactNode {
  if (k === 'cloud' && CLOUDS.has(value)) return <CloudIcon cloud={value as Cloud} size={size} />;
  if (k === 'cluster') return <BrandLogo name="kubernetes" size={size} />;
  return null;
}

const GroupShape = memo(function GroupShape({ g, mode, palette }: { g: PlacedGroup; mode: ViewMode; palette: Palette }) {
  const f = boxFaces(g.x, g.y, g.w, g.d, levelZ(g.level), SLAB, mode, UNIT);
  const c = groupColor(palette, g.group.key, g.level);
  return (
    <g className="cc-group">
      {f.left && <polygon points={f.left} fill={c.side} />}
      {f.right && <polygon points={f.right} fill={c.side} />}
      <polygon points={f.top} fill={c.top} stroke={c.stroke} strokeWidth={1} />
    </g>
  );
});

/** A group's name and size on its label strip, drawn above the blocks so they never hide it. */
const GroupLabel = memo(function GroupLabel({ g, mode, palette, label }: { g: PlacedGroup; mode: ViewMode; palette: Palette; label: string }) {
  const z = levelZ(g.level) + SLAB;
  const c = groupColor(palette, g.group.key, g.level);
  const fontSize = g.level === 0 ? 15 : 12;
  const mark = groupMark(g.group.key, g.group.value, fontSize + 3);
  return (
    <g transform={planeTransform(g.x + PAD * 0.75, g.y + g.d - HEADER * 0.9, z, mode, UNIT)} className="cc-group-label">
      {mark && <g transform={`translate(0 ${(HEADER * 0.55 * UNIT - fontSize) / 2 - 2})`}>{mark}</g>}
      <text x={mark ? fontSize + 9 : 0} y={HEADER * 0.55 * UNIT} fontSize={fontSize} fontWeight={600} fill={c.label}>
        {label}
        <tspan fill={palette.muted} fontWeight={400} dx={8}>
          {g.group.count.toLocaleString()}
        </tspan>
      </text>
    </g>
  );
});

function blockTitle(b: PlacedBlock): string {
  const n = b.node;
  const parts = [n.aggregate ? `${n.aggregate} more ${TYPE_LABELS[n.resourceType]} resources` : `${n.name} (${TYPE_LABELS[n.resourceType]})`];
  if (!n.aggregate) parts.push(`health: ${n.health}`);
  if (n.alerts) parts.push(`${n.alerts} monitor${n.alerts > 1 ? 's' : ''} alerting`);
  if (n.errorRate != null) parts.push(`error rate ${(n.errorRate * 100).toFixed(1)}%`);
  return parts.join(' · ');
}

const Block = memo(function Block({ b, mode, palette, overlay, selected, icons }: { b: PlacedBlock; mode: ViewMode; palette: Palette; overlay: CloudMapOverlay; selected: boolean; icons: boolean }) {
  const z = levelZ(b.level) + SLAB;
  const n = b.node;
  const base = blockColor(palette, n.resourceType, n.resource?.cloud, n.health, overlay);
  const c = faces(base);
  const h = n.aggregate ? BLOCK_HEIGHT * 0.45 : BLOCK_HEIGHT;
  const f = boxFaces(b.x, b.y, BLOCK, BLOCK, z, h, mode, UNIT);
  // Folded blocks are drawn as a short stack.
  const stack = n.aggregate ? [boxFaces(b.x, b.y, BLOCK, BLOCK, z + h, h * 0.6, mode, UNIT)] : [];
  const topZ = z + h + (n.aggregate ? h * 0.6 : 0);
  return (
    <g className={`cc-block${selected ? ' selected' : ''}${n.resource ? '' : ' folded'}`} data-id={b.id}>
      <title>{blockTitle(b)}</title>
      {[f, ...stack].map((x, i) => (
        <g key={i}>
          {x.left && <polygon points={x.left} fill={c.left} />}
          {x.right && <polygon points={x.right} fill={c.right} />}
          <polygon points={x.top} fill={c.top} stroke={selected ? palette.text : c.edge} strokeWidth={selected ? 2 : 0.6} />
        </g>
      ))}
      {icons && !n.aggregate && (
        <g transform={planeTransform(b.x + 0.2, b.y + 0.2, topZ, mode, UNIT)} color="#ffffff">
          <g transform={`scale(${(0.6 * UNIT) / 16})`}>
            <TypeIcon type={n.resourceType} size={16} />
          </g>
        </g>
      )}
      {n.aggregate != null && icons && (
        <g transform={planeTransform(b.x + 0.12, b.y + 0.62, topZ, mode, UNIT)}>
          <text fontSize={9} fontWeight={700} fill="#ffffff">
            +{n.aggregate}
          </text>
        </g>
      )}
      {n.alerts > 0 && <circle cx={project(b.x + 1, b.y, topZ, mode, UNIT).x} cy={project(b.x + 1, b.y, topZ, mode, UNIT).y} r={5} fill={palette.error} stroke="#fff" strokeWidth={1.5} />}
    </g>
  );
});

/** The diagram: platforms per group, blocks per resource, dependency arrows; pan by dragging, zoom with the wheel. */
export const CloudDiagram = forwardRef<DiagramHandle, Props>(function CloudDiagram({ layout, mode, overlay, edges, selectedId, onSelect, groupLabel }, ref) {
  const wrap = useRef<HTMLDivElement>(null);
  const svg = useRef<SVGSVGElement>(null);
  const [view, setView] = useState<Viewport>({ k: 1, tx: 0, ty: 0, width: 800, height: 600 });
  const palette = useMemo(readPalette, []);
  const bounds = useMemo(() => screenBounds(layout, mode, UNIT), [layout, mode]);
  const drag = useRef<{ x: number; y: number; tx: number; ty: number; moved: boolean } | null>(null);

  const fit = useCallback(() => {
    const el = wrap.current;
    if (!el) return;
    setView(fitView(bounds, el.clientWidth, el.clientHeight, 48, 1.4));
  }, [bounds]);

  // Fit whenever the diagram itself changes.
  useLayoutEffect(fit, [fit]);
  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setView((v) => ({ ...v, width: el.clientWidth, height: el.clientHeight })));
    ro.observe(el);
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = el.getBoundingClientRect();
      setView((v) => zoomAt(v, Math.exp(-e.deltaY * 0.0015), e.clientX - r.left, e.clientY - r.top));
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      ro.disconnect();
      el.removeEventListener('wheel', onWheel);
    };
  }, []);

  const groups = useMemo(() => paintOrder(layout.groups), [layout]);
  const blocks = useMemo(() => paintOrder(layout.blocks), [layout]);
  const icons = view.k >= ICON_ZOOM;
  const labelsOn = view.k >= LABEL_ZOOM;

  // The static scene only re-renders when the diagram, the overlay, the selection or the detail level change, not on pan.
  const scene = useMemo(
    () => (
      <>
        {groups.map((g) => (
          <GroupShape key={g.id} g={g} mode={mode} palette={palette} />
        ))}
        {blocks.map((b) => (
          <Block key={b.id} b={b} mode={mode} palette={palette} overlay={overlay} selected={b.id === selectedId} icons={icons} />
        ))}
        {groups.map((g) => (
          <GroupLabel key={g.id} g={g} mode={mode} palette={palette} label={groupLabel(g.group.key, g.group.value)} />
        ))}
      </>
    ),
    [groups, blocks, mode, palette, overlay, selectedId, icons, groupLabel],
  );

  const arrows = useMemo(() => {
    if (overlay !== 'dependencies' || !edges.length) return null;
    const anchors = serviceAnchors(layout.blocks);
    const top = (b: PlacedBlock) => project(b.x + BLOCK / 2, b.y + BLOCK / 2, levelZ(b.level) + SLAB + BLOCK_HEIGHT, mode, UNIT);
    const max = Math.max(...edges.map((e) => e.requests), 1);
    return edges.flatMap((e) => {
      const a = anchors.get(e.from);
      const b = anchors.get(e.to);
      if (!a || !b) return [];
      const p = top(a);
      const q = top(b);
      const mx = (p.x + q.x) / 2;
      const my = Math.min(p.y, q.y) - Math.max(30, Math.hypot(q.x - p.x, q.y - p.y) * 0.25);
      const bad = e.errorRate >= 0.05;
      return [
        <path
          key={`${e.from}>${e.to}`}
          d={`M${p.x},${p.y} Q${mx},${my} ${q.x},${q.y}`}
          fill="none"
          stroke={bad ? palette.error : palette.blue}
          strokeWidth={1.2 + 3 * Math.sqrt(e.requests / max)}
          strokeOpacity={0.85}
          markerEnd={bad ? 'url(#cc-arrow-bad)' : 'url(#cc-arrow)'}
        >
          <title>{`${e.from} → ${e.to}: ${e.requests.toLocaleString()} requests, ${(e.errorRate * 100).toFixed(1)}% errors (last hour)`}</title>
        </path>,
      ];
    });
  }, [overlay, edges, layout.blocks, mode, palette]);

  // Names only for blocks on screen when zoomed in, so thousands of resources stay fast.
  const labels = useMemo(() => {
    if (!labelsOn) return null;
    const out: ReactNode[] = [];
    for (const b of blocks) {
      if (out.length >= MAX_LABELS) break;
      const z = levelZ(b.level) + SLAB;
      const anchor = project(b.x + BLOCK / 2, b.y + BLOCK, z, mode, UNIT);
      if (!onScreen(anchor, view)) continue;
      const name = b.node.name.length > 22 ? `${b.node.name.slice(0, 21)}…` : b.node.name;
      out.push(
        <text key={b.id} x={anchor.x} y={anchor.y + (mode === 'iso' ? 12 : 11)} fontSize={8.5} textAnchor="middle" fill={palette.text} className="cc-label">
          {name}
        </text>,
      );
    }
    return out;
  }, [labelsOn, blocks, mode, view, palette]);

  useImperativeHandle(
    ref,
    () => ({
      zoomIn: () => setView((v) => zoomAt(v, 1.25, v.width / 2, v.height / 2)),
      zoomOut: () => setView((v) => zoomAt(v, 0.8, v.width / 2, v.height / 2)),
      fit,
      toSvg: () => {
        const el = svg.current;
        if (!el) return '';
        const pad = 24;
        const w = Math.ceil(bounds.maxX - bounds.minX + 2 * pad);
        const h = Math.ceil(bounds.maxY - bounds.minY + 2 * pad);
        const clone = el.cloneNode(true) as SVGSVGElement;
        clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
        clone.setAttribute('width', String(w));
        clone.setAttribute('height', String(h));
        clone.setAttribute('viewBox', `0 0 ${w} ${h}`);
        clone.querySelector('[data-viewport]')?.setAttribute('transform', `translate(${pad - bounds.minX} ${pad - bounds.minY})`);
        clone.querySelector('[data-labels]')?.remove();
        const bg = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
        bg.setAttribute('width', '100%');
        bg.setAttribute('height', '100%');
        bg.setAttribute('fill', palette.bg);
        clone.insertBefore(bg, clone.firstChild);
        const style = getComputedStyle(el);
        // Colours given as CSS variables (icons use currentColor) resolve to their values.
        return new XMLSerializer().serializeToString(clone).replace(/var\((--[\w-]+)\)/g, (_, name: string) => style.getPropertyValue(name).trim() || '#5a6273');
      },
    }),
    [fit, bounds, palette],
  );

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    drag.current = { x: e.clientX, y: e.clientY, tx: view.tx, ty: view.ty, moved: false };
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (!d.moved && Math.hypot(dx, dy) < 4) return;
    d.moved = true;
    setView((v) => ({ ...v, tx: d.tx + dx, ty: d.ty + dy }));
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const d = drag.current;
    drag.current = null;
    if (d?.moved) return;
    // A click: the block under the pointer, if any.
    const target = document.elementFromPoint(e.clientX, e.clientY)?.closest('[data-id]');
    const id = target?.getAttribute('data-id');
    onSelect(id ? (layout.blocks.find((b) => b.id === id) ?? null) : null);
  };

  return (
    <div ref={wrap} className="cc-canvas" onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp}>
      <svg ref={svg} width="100%" height="100%" role="img" aria-label="Architecture diagram">
        <defs>
          <marker id="cc-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
            <path d="M0,0 L10,5 L0,10 z" fill={palette.blue} />
          </marker>
          <marker id="cc-arrow-bad" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
            <path d="M0,0 L10,5 L0,10 z" fill={palette.error} />
          </marker>
        </defs>
        <g data-viewport transform={`translate(${view.tx} ${view.ty}) scale(${view.k})`}>
          {scene}
          {arrows}
          <g data-labels>{labels}</g>
        </g>
      </svg>
    </div>
  );
});
