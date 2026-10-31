// Rebuilds a recorded page from the browser SDK's records (full snapshots of the DOM, then incremental
// changes) inside a sandboxed iframe that runs no scripts. Event handlers, `javascript:` URLs and
// script elements of the recording are dropped; the node ids of the recording map to the rebuilt nodes.

type Rec = { type: number; timestamp: number; data?: Record<string, unknown> };
interface SNode {
  type: number;
  id: number;
  tagName?: string;
  name?: string;
  attributes?: Record<string, string | number | boolean>;
  childNodes?: SNode[];
  textContent?: string;
  isSVG?: boolean;
}

export const RecordType = { FullSnapshot: 2, Incremental: 3, Meta: 4, Focus: 6, ViewEnd: 7, VisualViewport: 8, Frustration: 9 } as const;
export const Source = { Mutation: 0, MouseMove: 1, MouseInteraction: 2, Scroll: 3, ViewportResize: 4, Input: 5, TouchMove: 6, MediaInteraction: 7, StyleSheetRule: 8 } as const;

const SVG_NS = 'http://www.w3.org/2000/svg';
const BLOCKED_TAGS = new Set(['script', 'noscript', 'iframe', 'object', 'embed', 'frame', 'frameset', 'base', 'meta']);

export interface Cursor {
  x: number;
  y: number;
}

export interface ReplayerCallbacks {
  onViewport?: (width: number, height: number) => void;
  onCursor?: (c: Cursor) => void;
  onClick?: (c: Cursor) => void;
}

/** The records of a recording, oldest first. */
export function flatten(segments: { records: unknown[] }[]): Rec[] {
  return segments.flatMap((s) => s.records as Rec[]).sort((a, b) => a.timestamp - b.timestamp);
}

/** Whether an attribute of the recording may be set on the rebuilt page. */
function safeAttribute(name: string, value: string): boolean {
  const n = name.toLowerCase();
  if (n.startsWith('on') || n === 'srcdoc' || n === 'formaction' || n === 'http-equiv') return false;
  if (['href', 'src', 'action', 'xlink:href', 'poster', 'data'].includes(n) && /^\s*(javascript|vbscript|data:text\/html)/i.test(value)) return false;
  return true;
}

export class Replayer {
  private nodes = new Map<number, Node>();
  private index = 0;
  readonly records: Rec[];

  constructor(
    private frame: HTMLIFrameElement,
    records: Rec[],
    private cb: ReplayerCallbacks = {},
  ) {
    this.records = records;
  }

  get start(): number {
    return this.records[0]?.timestamp ?? 0;
  }

  get end(): number {
    return this.records.at(-1)?.timestamp ?? 0;
  }

  /** Shows the page as it was at `t`: the latest full snapshot before it, then the changes since. */
  seek(t: number) {
    let full = -1;
    for (let i = 0; i < this.records.length && this.records[i].timestamp <= t; i++) if (this.records[i].type === RecordType.FullSnapshot) full = i;
    if (full < 0) full = this.records.findIndex((r) => r.type === RecordType.FullSnapshot);
    if (full < 0) return;
    // The Meta record before the snapshot gives the viewport.
    for (let i = full; i >= 0; i--)
      if (this.records[i].type === RecordType.Meta) {
        this.apply(this.records[i]);
        break;
      }
    this.index = full;
    this.advance(Math.max(t, this.records[full].timestamp));
  }

  /** Applies the records up to `t` (time moves forward). */
  advance(t: number) {
    while (this.index < this.records.length && this.records[this.index].timestamp <= t) this.apply(this.records[this.index++]);
  }

  private doc(): Document | null {
    return this.frame.contentDocument;
  }

  private apply(r: Rec) {
    const d = (r.data ?? {}) as Record<string, unknown>;
    switch (r.type) {
      case RecordType.Meta:
        this.cb.onViewport?.(Number(d.width) || 1280, Number(d.height) || 800);
        break;
      case RecordType.FullSnapshot:
        this.rebuild(d.node as SNode);
        break;
      case RecordType.Incremental:
        this.incremental(d);
        break;
    }
  }

  private rebuild(root: SNode) {
    const doc = this.doc();
    if (!doc || !root) return;
    this.nodes.clear();
    // Swapping the root element (not document.open) keeps the iframe out of the session history.
    const html = (root.childNodes ?? []).find((n) => n.type === 2);
    const built = html && this.build(html, false);
    if (!built) return;
    if (doc.documentElement) doc.replaceChild(built, doc.documentElement);
    else doc.appendChild(built);
    this.nodes.set(root.id, doc);
  }

  private build(n: SNode, svg: boolean): Node | null {
    const doc = this.doc()!;
    let node: Node | null = null;
    switch (n.type) {
      case 2: {
        const tag = (n.tagName ?? 'div').toLowerCase();
        if (BLOCKED_TAGS.has(tag)) return null;
        const isSvg = svg || !!n.isSVG || tag === 'svg';
        const el = isSvg ? doc.createElementNS(SVG_NS, tag) : doc.createElement(tag === 'link' ? 'style' : tag);
        for (const [k, raw] of Object.entries(n.attributes ?? {})) {
          const v = String(raw);
          // The SDK inlines stylesheets as `_cssText`; linked stylesheets aren't fetched.
          if (k === '_cssText') {
            el.textContent = v;
            continue;
          }
          if (k.startsWith('rr_') || !safeAttribute(k, v)) continue;
          try {
            el.setAttribute(k, v);
          } catch {
            /* invalid attribute name */
          }
        }
        if (tag === 'a') el.removeAttribute('target');
        for (const c of n.childNodes ?? []) {
          const child = this.build(c, isSvg);
          if (child) el.appendChild(child);
        }
        node = el;
        break;
      }
      case 3:
      case 4:
        node = doc.createTextNode(n.textContent ?? '');
        break;
      default:
        return null;
    }
    this.nodes.set(n.id, node);
    return node;
  }

  private incremental(d: Record<string, unknown>) {
    switch (d.source) {
      case Source.Mutation:
        this.mutate(d);
        break;
      case Source.MouseMove:
      case Source.TouchMove: {
        const last = (d.positions as Cursor[] | undefined)?.at(-1);
        if (last) this.cb.onCursor?.({ x: last.x, y: last.y });
        break;
      }
      case Source.MouseInteraction:
        // 2 = click (0 mouseup, 1 mousedown, …).
        if (d.type === 2 && typeof d.x === 'number') this.cb.onClick?.({ x: d.x as number, y: d.y as number });
        if (typeof d.x === 'number') this.cb.onCursor?.({ x: d.x as number, y: d.y as number });
        break;
      case Source.Scroll: {
        const target = this.nodes.get(Number(d.id));
        const doc = this.doc();
        if (target === doc || target === doc?.documentElement || target === doc?.body) doc?.defaultView?.scrollTo(Number(d.x), Number(d.y));
        else if (target instanceof Element) target.scrollTo(Number(d.x), Number(d.y));
        break;
      }
      case Source.ViewportResize:
        this.cb.onViewport?.(Number(d.width) || 1280, Number(d.height) || 800);
        break;
      case Source.Input: {
        const el = this.nodes.get(Number(d.id));
        if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
          if (typeof d.text === 'string') el.value = d.text;
          if (typeof d.isChecked === 'boolean' && el instanceof HTMLInputElement) el.checked = d.isChecked;
        }
        break;
      }
    }
  }

  private mutate(d: Record<string, unknown>) {
    for (const r of (d.removes as { id: number }[]) ?? []) {
      const n = this.nodes.get(r.id);
      n?.parentNode?.removeChild(n);
      this.nodes.delete(r.id);
    }
    for (const a of (d.adds as { parentId: number; nextId: number | null; node: SNode }[]) ?? []) {
      const parent = this.nodes.get(a.parentId);
      if (!parent) continue;
      const svg = parent instanceof SVGElement;
      const node = this.build(a.node, svg);
      if (!node) continue;
      const next = a.nextId != null ? this.nodes.get(a.nextId) : null;
      parent.insertBefore(node, next && next.parentNode === parent ? next : null);
    }
    for (const t of (d.texts as { id: number; value: string | null }[]) ?? []) {
      const n = this.nodes.get(t.id);
      if (n) n.textContent = t.value ?? '';
    }
    for (const a of (d.attributes as { id: number; attributes: Record<string, string | null> }[]) ?? []) {
      const el = this.nodes.get(a.id);
      if (!(el instanceof Element)) continue;
      for (const [k, v] of Object.entries(a.attributes)) {
        if (v == null) el.removeAttribute(k);
        else if (k === '_cssText') el.textContent = v;
        else if (safeAttribute(k, v))
          try {
            el.setAttribute(k, v);
          } catch {
            /* invalid attribute name */
          }
      }
    }
  }
}
