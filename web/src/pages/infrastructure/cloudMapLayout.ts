// Cloudcraft diagram layout: packs the server's group tree onto a ground plane (groups as nested
// platforms, resources as blocks) and projects it isometrically or flat (2D). Pure functions, so the
// renderer only draws; units are grid cells, the renderer scales them.
import type { CloudMapGroup, CloudMapNode } from '../../api/cloudMap/client';

/** Block footprint, the gap between blocks, a group's padding and its label strip (along the front edge, where nothing stands in front of it). */
export const BLOCK = 1;
export const GAP = 0.7;
export const PAD = 0.8;
export const HEADER = 1.7;
/** Space between sibling groups. */
export const SPACING = 1.2;
/** Height of a group's platform and of each nesting level above its parent. */
export const SLAB = 0.18;
export const BLOCK_HEIGHT = 0.9;

export type ViewMode = 'iso' | 'flat';

export interface PlacedGroup {
  id: string;
  group: CloudMapGroup;
  /** Ground-plane rectangle. */
  x: number;
  y: number;
  w: number;
  d: number;
  /** Nesting depth (top-level groups are 0). */
  level: number;
}

export interface PlacedBlock {
  id: string;
  node: CloudMapNode;
  x: number;
  y: number;
  /** Level of the group it stands on. */
  level: number;
}

export interface DiagramLayout {
  groups: PlacedGroup[];
  blocks: PlacedBlock[];
  w: number;
  d: number;
}

interface Measured {
  w: number;
  d: number;
  /** Places the measured box with its top-left corner at (x, y). */
  place: (x: number, y: number, out: DiagramLayout) => void;
}

const PITCH = BLOCK + GAP;

/** Columns for `n` blocks: a near-square grid, a little wider than deep. */
export function gridColumns(n: number): number {
  return n <= 0 ? 0 : Math.min(n, Math.ceil(Math.sqrt(n * 1.4)));
}

/** Rows of boxes no wider than `maxWidth` (each row as wide as it gets), in order. */
export function shelfPack(sizes: { w: number; d: number }[], maxWidth: number, spacing = SPACING): { positions: { x: number; y: number }[]; w: number; d: number } {
  const positions: { x: number; y: number }[] = [];
  let x = 0;
  let y = 0;
  let rowDepth = 0;
  let width = 0;
  for (const s of sizes) {
    if (x > 0 && x + s.w > maxWidth) {
      y += rowDepth + spacing;
      x = 0;
      rowDepth = 0;
    }
    positions.push({ x, y });
    x += s.w + spacing;
    width = Math.max(width, x - spacing);
    rowDepth = Math.max(rowDepth, s.d);
  }
  return { positions, w: width, d: sizes.length ? y + rowDepth : 0 };
}

/** Room the label strip needs: the name, the count and a logo, in grid units (about 0.3 per character). */
export function labelWidth(g: CloudMapGroup): number {
  return 2 * PAD + 1 + (String(g.value || g.key).length + String(g.count).length + 2) * 0.3;
}

function measure(g: CloudMapGroup, level: number, path: string): Measured {
  const id = `${path}/${g.key}:${g.value}`;
  if (!g.groups.length) {
    const cols = gridColumns(g.nodes.length);
    const rows = cols ? Math.ceil(g.nodes.length / cols) : 0;
    const innerW = Math.max(cols * PITCH - GAP, BLOCK);
    const innerD = Math.max(rows * PITCH - GAP, BLOCK);
    const w = Math.max(innerW + 2 * PAD, labelWidth(g));
    const d = innerD + HEADER + PAD;
    return {
      w,
      d,
      place: (x, y, out) => {
        out.groups.push({ id, group: g, x, y, w, d, level });
        g.nodes.forEach((node, i) => out.blocks.push({ id: node.id, node, x: x + PAD + (i % cols) * PITCH, y: y + PAD + Math.floor(i / cols) * PITCH, level }));
      },
    };
  }
  const children = g.groups.map((c) => measure(c, level + 1, id));
  const area = children.reduce((s, c) => s + c.w * c.d, 0);
  const maxWidth = Math.max(...children.map((c) => c.w), Math.sqrt(area) * 1.5);
  const packed = shelfPack(children, maxWidth);
  const w = Math.max(packed.w + 2 * PAD, labelWidth(g));
  const d = packed.d + HEADER + PAD;
  return {
    w,
    d,
    place: (x, y, out) => {
      out.groups.push({ id, group: g, x, y, w, d, level });
      children.forEach((c, i) => c.place(x + PAD + packed.positions[i].x, y + PAD + packed.positions[i].y, out));
    },
  };
}

/** Lays out the tree: top-level groups side by side (the root itself is not drawn). */
export function layoutDiagram(root: CloudMapGroup): DiagramLayout {
  const out: DiagramLayout = { groups: [], blocks: [], w: 0, d: 0 };
  if (!root.groups.length) {
    // Ungrouped: the root's blocks on one unnamed platform.
    if (root.nodes.length) {
      const m = measure({ ...root, key: 'all', value: '' }, 0, '');
      m.place(0, 0, out);
      out.w = m.w;
      out.d = m.d;
    }
    return out;
  }
  const tops = root.groups.map((g) => measure(g, 0, ''));
  const area = tops.reduce((s, c) => s + c.w * c.d, 0);
  const packed = shelfPack(tops, Math.max(...tops.map((t) => t.w), Math.sqrt(area) * 1.6), SPACING * 2);
  tops.forEach((t, i) => t.place(packed.positions[i].x, packed.positions[i].y, out));
  out.w = packed.w;
  out.d = packed.d;
  return out;
}

const COS = Math.cos(Math.PI / 6);
const SIN = Math.sin(Math.PI / 6);

/** Screen point of a ground-plane point at height z (all in grid units, scaled by `unit`). */
export function project(x: number, y: number, z: number, mode: ViewMode, unit: number): { x: number; y: number } {
  if (mode === 'flat') return { x: x * unit, y: y * unit };
  return { x: (x - y) * COS * unit, y: ((x + y) * SIN - z) * unit };
}

/** SVG transform that paints flat content (labels, icons) onto the ground plane at (x, y, z). */
export function planeTransform(x: number, y: number, z: number, mode: ViewMode, unit: number): string {
  const p = project(x, y, z, mode, unit);
  if (mode === 'flat') return `translate(${round(p.x)} ${round(p.y)})`;
  return `matrix(${round(COS)} ${round(SIN)} ${round(-COS)} ${round(SIN)} ${round(p.x)} ${round(p.y)})`;
}

const round = (v: number) => Math.round(v * 100) / 100;
const pts = (points: { x: number; y: number }[]) => points.map((p) => `${round(p.x)},${round(p.y)}`).join(' ');

/** Faces of a box (ground rectangle x, y, w, d from z to z + h): `top`, and the visible `left` and `right` sides in iso. */
export function boxFaces(x: number, y: number, w: number, d: number, z: number, h: number, mode: ViewMode, unit: number): { top: string; left: string | null; right: string | null } {
  const P = (px: number, py: number, pz: number) => project(px, py, pz, mode, unit);
  const top = pts([P(x, y, z + h), P(x + w, y, z + h), P(x + w, y + d, z + h), P(x, y + d, z + h)]);
  if (mode === 'flat') return { top, left: null, right: null };
  // Visible sides face the viewer: the front-left (y + d) and front-right (x + w) edges.
  const left = pts([P(x, y + d, z + h), P(x + w, y + d, z + h), P(x + w, y + d, z), P(x, y + d, z)]);
  const right = pts([P(x + w, y, z + h), P(x + w, y + d, z + h), P(x + w, y + d, z), P(x + w, y, z)]);
  return { top, left, right };
}

/** Height where a group of `level` starts (each level stands on its parent's platform). */
export const levelZ = (level: number) => level * SLAB;

/** Screen bounds of the layout. */
export function screenBounds(layout: DiagramLayout, mode: ViewMode, unit: number): { minX: number; minY: number; maxX: number; maxY: number } {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  const add = (x: number, y: number, z: number) => {
    const p = project(x, y, z, mode, unit);
    minX = Math.min(minX, p.x);
    maxX = Math.max(maxX, p.x);
    minY = Math.min(minY, p.y);
    maxY = Math.max(maxY, p.y);
  };
  for (const g of layout.groups) {
    const z = levelZ(g.level);
    for (const [x, y] of [
      [g.x, g.y],
      [g.x + g.w, g.y],
      [g.x, g.y + g.d],
      [g.x + g.w, g.y + g.d],
    ])
      for (const h of [z, z + SLAB + BLOCK_HEIGHT]) add(x, y, h);
  }
  if (minX === Infinity) return { minX: 0, minY: 0, maxX: 0, maxY: 0 };
  return { minX, minY, maxX, maxY };
}

/** Back-to-front order for the painter's algorithm (blocks further from the viewer first). */
export function paintOrder<T extends { x: number; y: number; level: number }>(items: T[]): T[] {
  return [...items].sort((a, b) => a.level - b.level || a.x + a.y - (b.x + b.y) || a.y - b.y);
}

/** The block that stands for each service (a workload or service first), for dependency arrows. */
export function serviceAnchors(blocks: PlacedBlock[]): Map<string, PlacedBlock> {
  const rank = (b: PlacedBlock) => (['kube_deployment', 'kube_statefulset', 'ecs_service', 'kube_daemonset'].includes(b.node.resourceType) ? 0 : b.node.resourceType === 'kube_service' ? 1 : 2);
  const out = new Map<string, PlacedBlock>();
  for (const b of blocks) {
    const s = b.node.service;
    if (!s) continue;
    const cur = out.get(s);
    if (!cur || rank(b) < rank(cur)) out.set(s, b);
  }
  return out;
}

export interface Viewport {
  /** Zoom and pan: screen = diagram * k + (tx, ty). */
  k: number;
  tx: number;
  ty: number;
  width: number;
  height: number;
}

/** Whether a diagram point is on screen (with a margin in screen pixels): only those get labels. */
export function onScreen(p: { x: number; y: number }, v: Viewport, margin = 40): boolean {
  const sx = p.x * v.k + v.tx;
  const sy = p.y * v.k + v.ty;
  return sx >= -margin && sy >= -margin && sx <= v.width + margin && sy <= v.height + margin;
}

/** Zoom and pan that fit `bounds` into a width × height viewport with padding, capped at `maxK`. */
export function fitView(bounds: { minX: number; minY: number; maxX: number; maxY: number }, width: number, height: number, padding = 40, maxK = 1.5): Viewport {
  const bw = Math.max(bounds.maxX - bounds.minX, 1);
  const bh = Math.max(bounds.maxY - bounds.minY, 1);
  const k = Math.min(maxK, Math.max(0.05, Math.min((width - 2 * padding) / bw, (height - 2 * padding) / bh)));
  return { k, tx: (width - bw * k) / 2 - bounds.minX * k, ty: (height - bh * k) / 2 - bounds.minY * k, width, height };
}

/** Zooms by `factor` around the screen point (px, py), keeping it under the cursor. */
export function zoomAt(v: Viewport, factor: number, px: number, py: number, min = 0.05, max = 6): Viewport {
  const k = Math.min(max, Math.max(min, v.k * factor));
  const f = k / v.k;
  return { ...v, k, tx: px - (px - v.tx) * f, ty: py - (py - v.ty) * f };
}
