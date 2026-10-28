// Workflow canvas layout: where the trigger, the steps, the connectors and the "+" insertion points
// go. A vertical flow centred on its column; if/else branches side by side (true left, false right)
// merging below; a "for each" body framed under its step. Pure, so it is tested without a browser.
import type { BlockRef } from '../../../api/workflows/logic';
import type { WorkflowStep, WorkflowStepKind } from '../../../api/workflows/types';

export const NODE_W = 264;
export const NODE_H = 60;
/** Vertical space between two nodes (a "+" sits in the middle). */
export const GAP_Y = 52;
/** Horizontal space between branches. */
export const GAP_X = 48;
/** Width an empty branch takes. */
export const EMPTY_W = 140;
/** Padding of a "for each" frame. */
export const FRAME_PAD = 20;
const SPLIT = 26;
const MERGE = 22;

export const TRIGGER_ID = '__trigger';

export interface LaidNode {
  id: string;
  kind: WorkflowStepKind | 'trigger';
  x: number;
  y: number;
}
export interface LaidAdd {
  x: number;
  y: number;
  ref: BlockRef;
  index: number;
}
export interface LaidLabel {
  x: number;
  y: number;
  text: string;
}
export interface LaidFrame {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
}
export interface Layout {
  nodes: LaidNode[];
  /** Polylines, each a list of [x, y]. */
  edges: [number, number][][];
  adds: LaidAdd[];
  labels: LaidLabel[];
  frames: LaidFrame[];
  width: number;
  height: number;
}

function blockWidth(list: WorkflowStep[]): number {
  if (list.length === 0) return EMPTY_W;
  return Math.max(...list.map(itemWidth));
}

function itemWidth(s: WorkflowStep): number {
  if (s.kind === 'if') return Math.max(NODE_W, blockWidth(s.steps) + GAP_X + blockWidth(s.elseSteps));
  if (s.kind === 'for_each') return Math.max(NODE_W, blockWidth(s.steps)) + 2 * FRAME_PAD;
  return NODE_W;
}

/** Lays a block out from its entry point (cx, y); returns the y where its flow leaves. */
function block(list: WorkflowStep[], ref: BlockRef, cx: number, y: number, out: Layout): number {
  list.forEach((s, index) => {
    out.edges.push([
      [cx, y],
      [cx, y + GAP_Y],
    ]);
    out.adds.push({ x: cx, y: y + GAP_Y / 2, ref, index });
    const top = y + GAP_Y;
    out.nodes.push({ id: s.id, kind: s.kind, x: cx - NODE_W / 2, y: top });
    const bottom = top + NODE_H;
    if (s.kind === 'if') {
      const wt = blockWidth(s.steps);
      const we = blockWidth(s.elseSteps);
      const total = wt + GAP_X + we;
      const lx = cx - total / 2 + wt / 2;
      const rx = cx + total / 2 - we / 2;
      const split = bottom + SPLIT;
      out.edges.push([
        [cx, bottom],
        [cx, split],
      ]);
      out.edges.push([
        [lx, split],
        [rx, split],
      ]);
      out.labels.push({ x: lx, y: split, text: 'true' }, { x: rx, y: split, text: 'false' });
      const endL = block(s.steps, { parent: s.id, branch: 'steps' }, lx, split, out);
      const endR = block(s.elseSteps, { parent: s.id, branch: 'elseSteps' }, rx, split, out);
      const merge = Math.max(endL, endR) + MERGE;
      out.edges.push([
        [lx, endL],
        [lx, merge],
        [rx, merge],
        [rx, endR],
      ]);
      y = merge;
    } else if (s.kind === 'for_each') {
      const fw = itemWidth(s);
      const end = block(s.steps, { parent: s.id, branch: 'steps' }, cx, bottom, out);
      out.frames.push({ id: s.id, x: cx - fw / 2, y: top - FRAME_PAD / 2, w: fw, h: end - top + FRAME_PAD });
      y = end + FRAME_PAD / 2;
    } else {
      y = bottom;
    }
  });
  // The trailing "+" of the block.
  out.edges.push([
    [cx, y],
    [cx, y + GAP_Y / 2 + 10],
  ]);
  out.adds.push({ x: cx, y: y + GAP_Y / 2, ref, index: list.length });
  return y + GAP_Y / 2 + 10;
}

/** The whole canvas: the trigger node, then the top-level steps; coordinates start at (0, 0). */
export function layoutWorkflow(steps: WorkflowStep[]): Layout {
  const width = Math.max(NODE_W, blockWidth(steps));
  const out: Layout = { nodes: [], edges: [], adds: [], labels: [], frames: [], width, height: 0 };
  const cx = width / 2;
  out.nodes.push({ id: TRIGGER_ID, kind: 'trigger', x: cx - NODE_W / 2, y: 0 });
  const end = block(steps, { parent: null, branch: 'steps' }, cx, NODE_H, out);
  out.height = end;
  return out;
}
