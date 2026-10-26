import { useMemo, useState } from 'react';
import { ChevronRight, CircleHelp } from 'lucide-react';
import type { FlameNode } from '../../../api/profiles/client';
import { fmtValue, frameLabel, framePackage, isLibrary } from './format';

/** Group by: each function on its own row, or library frames folded into their package. */
export type GroupBy = 'function' | 'package';

/** A frame as drawn: library frames can be folded into one "package" row. */
export interface ViewNode {
  key: string;
  name: string;
  file: string;
  group: boolean;
  value: number;
  children: ViewNode[];
}

function merge(into: ViewNode, from: ViewNode) {
  into.value += from.value;
  for (const c of from.children) {
    const same = into.children.find((x) => x.key === c.key);
    if (same) merge(same, c);
    else into.children.push(c);
  }
}

/**
 * The tree as drawn. Unless `expandAll`, consecutive frames of one library (stdlib, a framework,
 * a runtime) fold into a single italic row, as Datadog does; grouping by package also folds
 * application frames by their package.
 */
export function viewTree(root: FlameNode, expandAll: boolean, groupBy: GroupBy): ViewNode {
  const packageOf = (n: FlameNode) => {
    const p = framePackage(n.name, n.file);
    if (groupBy === 'package') return p || n.file || n.name;
    return isLibrary(p, n.file) ? p : '';
  };
  const children = (nodes: FlameNode[], parentKey: string): ViewNode[] => {
    const out: ViewNode[] = [];
    const add = (v: ViewNode) => {
      const same = out.find((x) => x.key === v.key);
      if (same) merge(same, v);
      else out.push(v);
    };
    for (const n of nodes) {
      const p = expandAll ? '' : packageOf(n);
      if (!p) {
        const key = `${parentKey};${n.name}@${n.file}`;
        add({ key, name: n.name, file: n.file, group: false, value: n.value, children: children(n.children, key) });
        continue;
      }
      // Walk down while frames stay in the package; what leaves it hangs under the folded row.
      const key = `${parentKey};[${p}]`;
      const leaving: FlameNode[] = [];
      const walk = (x: FlameNode) => {
        for (const c of x.children) {
          if (packageOf(c) === p) walk(c);
          else leaving.push(c);
        }
      };
      walk(n);
      add({ key, name: p, file: '', group: true, value: n.value, children: children(leaving, key) });
    }
    out.sort((a, b) => b.value - a.value);
    return out;
  };
  return { key: 'all', name: 'all', file: '', group: false, value: root.value, children: children(root.children, 'all') };
}

/** Per-minute values of every drawn frame, by key (for comparisons). */
export function valuesByKey(v: ViewNode, scale: number, out = new Map<string, number>()): Map<string, number> {
  out.set(v.key, (out.get(v.key) ?? 0) + v.value * scale);
  v.children.forEach((c) => valuesByKey(c, scale, out));
  return out;
}

interface Box {
  node: ViewNode;
  depth: number;
  x: number;
  w: number;
}

/** Frames narrower than this share of the graph aren't drawn. */
const MIN_WIDTH = 0.0015;
const ROW = 24;

function layout(focus: ViewNode): Box[] {
  const out: Box[] = [];
  const total = focus.value || 1;
  const visit = (n: ViewNode, depth: number, x: number) => {
    let cx = x;
    for (const c of n.children) {
      const w = c.value / total;
      if (w < MIN_WIDTH) {
        cx += w;
        continue;
      }
      out.push({ node: c, depth, x: cx, w });
      visit(c, depth + 1, cx);
      cx += w;
    }
  };
  visit(focus, 0, 0);
  return out;
}

function find(n: ViewNode, key: string): ViewNode | null {
  if (n.key === key) return n;
  for (const c of n.children) {
    const f = find(c, key);
    if (f) return f;
  }
  return null;
}

export interface Diff {
  /** The other side's per-minute values by frame key. */
  other: Map<string, number>;
  side: 'A' | 'B';
}

interface Props {
  tree: ViewNode;
  /** Raw value → displayed value (per minute, or average for gauges). */
  scale: number;
  unit: string;
  title: string;
  filter?: string;
  diff?: Diff;
}

/** Datadog's flame graph: root at the top, callees below, width = share of the metric. */
export function FlameGraphView({ tree, scale, unit, title, filter = '', diff }: Props) {
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const focus = (focusKey && find(tree, focusKey)) || tree;
  const boxes = useMemo(() => layout(focus), [focus]);
  const depth = boxes.reduce((d, b) => Math.max(d, b.depth + 1), 0);
  const total = tree.value || 1;
  const needle = filter.trim().toLowerCase();

  const style = (n: ViewNode) => {
    if (diff) {
      const mine = n.value * scale;
      const theirs = diff.other.get(n.key);
      if (theirs == null) return { background: diff.side === 'A' ? 'var(--side-a)' : 'var(--side-b)', color: '#fff' };
      if (diff.side === 'A') return {};
      const delta = (mine - theirs) / Math.max(mine, theirs, 1e-9);
      const pct = Math.round(Math.min(1, Math.abs(delta)) * 85 + 10);
      const tone = delta >= 0 ? 'var(--status-error)' : 'var(--status-ok)';
      return { background: `color-mix(in srgb, ${tone} ${pct}%, var(--bg))`, color: pct > 45 ? '#fff' : undefined };
    }
    if (n.group) return {};
    const share = Math.sqrt(n.value / total);
    const pct = Math.round(15 + share * 85);
    return { background: `color-mix(in srgb, var(--flame) ${pct}%, var(--bg))`, color: pct > 50 ? '#fff' : undefined };
  };

  return (
    <div className="pf-flame">
      <div className="pf-flame-title" onClick={() => setFocusKey(null)} title={focusKey ? 'Reset zoom' : undefined}>
        <span>
          {title}: {fmtValue(focus.value * scale, unit)}
        </span>
        <CircleHelp size={16} className="pf-flame-help" aria-label="Click a frame to zoom into it; click this bar to zoom out." />
      </div>
      <div className="pf-flame-body" style={{ height: depth * ROW }}>
        {boxes.map(({ node, depth: d, x, w }) => {
          const label = node.group ? node.name : frameLabel(node.name, node.file);
          const dim = needle && !label.toLowerCase().includes(needle);
          return (
            <div
              key={node.key}
              className={`pf-frame${node.group ? ' pf-frame-group' : ''}${dim ? ' pf-dim' : ''}`}
              style={{ top: d * ROW, left: `${x * 100}%`, width: `${w * 100}%`, ...style(node) }}
              title={`${label}${node.file && !node.group ? `\n${node.file}` : ''}\n${fmtValue(node.value * scale, unit)} (${((node.value / total) * 100).toFixed(1)}%)`}
              onClick={() => setFocusKey(node.key)}
            >
              {w * 100 > 1.2 && (
                <span className="pf-frame-label">
                  {node.group && <ChevronRight size={12} />}
                  {label}
                </span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
