// Layered left-to-right layout of the Data Streams map: producers on the left, each topic one
// column after the services publishing to it, each consumer one column after its topics.

export interface LayoutEdge {
  source: string;
  target: string;
}

export interface Placed {
  id: string;
  layer: number;
  order: number;
  /** Center of the node. */
  x: number;
  y: number;
}

export interface Layout {
  nodes: Map<string, Placed>;
  width: number;
  height: number;
}

export interface LayoutOptions {
  columnGap: number;
  rowGap: number;
  margin: number;
}

const DEFAULTS: LayoutOptions = { columnGap: 250, rowGap: 96, margin: 110 };

/**
 * Layer of each node: the longest path to it from a node nobody points at. Edges that close a
 * cycle are ignored; nodes only on cycles start where the walk first reaches them.
 */
export function layers(ids: string[], edges: LayoutEdge[]): Map<string, number> {
  const known = new Set(ids);
  const out = new Map<string, string[]>();
  const incoming = new Map<string, number>(ids.map((id) => [id, 0]));
  for (const e of edges) {
    if (!known.has(e.source) || !known.has(e.target) || e.source === e.target) continue;
    out.set(e.source, [...(out.get(e.source) ?? []), e.target]);
    incoming.set(e.target, (incoming.get(e.target) ?? 0) + 1);
  }
  // Depth-first from the roots, keeping only edges that don't point back into the current walk.
  const forward: LayoutEdge[] = [];
  const state = new Map<string, 'open' | 'done'>();
  const visit = (id: string) => {
    state.set(id, 'open');
    for (const t of [...(out.get(id) ?? [])].sort()) {
      const s = state.get(t);
      if (s === 'open') continue;
      forward.push({ source: id, target: t });
      if (!s) visit(t);
    }
    state.set(id, 'done');
  };
  const sorted = [...ids].sort();
  for (const id of sorted.filter((i) => !incoming.get(i))) visit(id);
  for (const id of sorted) if (!state.has(id)) visit(id);
  // Longest path over the acyclic forward edges.
  const layer = new Map<string, number>(ids.map((id) => [id, 0]));
  for (let changed = true, rounds = 0; changed && rounds <= ids.length; rounds++) {
    changed = false;
    for (const e of forward) {
      const want = layer.get(e.source)! + 1;
      if (want > layer.get(e.target)!) {
        layer.set(e.target, want);
        changed = true;
      }
    }
  }
  return layer;
}

/** Positions: columns by layer, nodes in a column ordered by the average row of what feeds them. */
export function layoutGraph(ids: string[], edges: LayoutEdge[], options: Partial<LayoutOptions> = {}): Layout {
  const o = { ...DEFAULTS, ...options };
  const layer = layers(ids, edges);
  const columns: string[][] = [];
  for (const id of [...ids].sort()) {
    const l = layer.get(id) ?? 0;
    (columns[l] ??= []).push(id);
  }
  const order = new Map<string, number>();
  columns.forEach((col, l) => {
    if (l === 0) {
      col.forEach((id, i) => order.set(id, i));
      return;
    }
    const score = (id: string) => {
      const feeds = edges.filter((e) => e.target === id && order.has(e.source)).map((e) => order.get(e.source)!);
      return feeds.length ? feeds.reduce((a, b) => a + b, 0) / feeds.length : Number.MAX_SAFE_INTEGER;
    };
    col.sort((a, b) => score(a) - score(b) || a.localeCompare(b)).forEach((id, i) => order.set(id, i));
  });
  const tallest = Math.max(1, ...columns.map((c) => c?.length ?? 0));
  const height = o.margin * 2 + (tallest - 1) * o.rowGap;
  const nodes = new Map<string, Placed>();
  columns.forEach((col, l) => {
    const offset = ((tallest - col.length) * o.rowGap) / 2;
    col.forEach((id, i) => nodes.set(id, { id, layer: l, order: i, x: o.margin + l * o.columnGap, y: o.margin + offset + i * o.rowGap }));
  });
  const width = o.margin * 2 + Math.max(0, columns.length - 1) * o.columnGap;
  return { nodes, width, height };
}

/**
 * SVG path of an edge between two node centers, leaving the source's right side and entering the
 * target's left side (`half` is half a node's width). Edges that go back left loop below the nodes.
 */
export function edgePath(a: { x: number; y: number }, b: { x: number; y: number }, half: number): string {
  const sx = a.x + half;
  const tx = b.x - half;
  if (tx > sx) {
    const c = (tx - sx) / 2;
    return `M${sx},${a.y} C${sx + c},${a.y} ${tx - c},${b.y} ${tx},${b.y}`;
  }
  const drop = Math.max(a.y, b.y) + 70;
  return `M${sx},${a.y} C${sx + 80},${a.y} ${sx + 80},${drop} ${(sx + tx) / 2},${drop} S${tx - 80},${b.y} ${tx},${b.y}`;
}

/** Where to put an edge's label: halfway between the two nodes. */
export function edgeMid(a: { x: number; y: number }, b: { x: number; y: number }): { x: number; y: number } {
  if (b.x > a.x) return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
  return { x: (a.x + b.x) / 2, y: Math.max(a.y, b.y) + 70 };
}
