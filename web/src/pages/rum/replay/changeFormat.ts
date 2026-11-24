// Reads the browser SDK's compact recording format (full snapshots with `format: 1`, change records of type 12)
// into the records the player rebuilds pages from: a node tree for full snapshots, mutation, input and scroll
// records for the changes. The format is the one described by the SDK's session replay schema
// (DataDog/browser-sdk, packages/browser-rum/src/types/sessionReplay.ts, Apache-2.0): strings are indexes into a
// table the records define as they go, node ids are implied by the order nodes are added (from 0 at each full
// snapshot) and a node's position is relative to the id it gets. Changes the player doesn't show (element sizes,
// media playback, the visual viewport, image contents) are left out.

export type Rec = { type: number; timestamp: number; format?: number; data?: unknown };

export interface SNode {
  type: number;
  id: number;
  tagName?: string;
  name?: string;
  attributes?: Record<string, string>;
  childNodes?: SNode[];
  textContent?: string;
  isSVG?: boolean;
}

interface Add {
  parentId: number;
  nextId: number | null;
  node: SNode;
}

const FULL_SNAPSHOT = 2;
const INCREMENTAL = 3;
const CHANGE = 12;
const CHANGE_FORMAT = 1;

const Change = {
  AddString: 0,
  AddNode: 1,
  RemoveNode: 2,
  Attribute: 3,
  Text: 4,
  ScrollPosition: 6,
  AddStyleSheet: 7,
  AttachedStyleSheets: 8,
  AddStrings: 11,
  InputValue: 12,
  InputSelection: 13,
  ClearStrings: 14,
} as const;

type Str = string | number | { string: string };
type Item = unknown[];

class UnknownString extends Error {}

/** The records with those of the change format read into the older format; the others are kept as they are. */
export function fromChangeFormat(records: Rec[]): Rec[] {
  const reader = new ChangeReader();
  return records.flatMap((r) => reader.read(r));
}

class ChangeReader {
  private strings: string[] = [];
  private nextNode = 0;
  private sheets: string[] = [];
  private parentOf = new Map<number, number>();
  private childrenOf = new Map<number, number[]>();
  // Shadow roots and document fragments: their children are shown in the node they belong to.
  private shownIn = new Map<number, number>();
  private names = new Map<number, string>();
  // The <style> elements standing for the stylesheets a document or shadow root adopted, by that node.
  private adopted = new Map<number, number>();
  private nextSynthetic = -1;
  // Change records build on the latest full snapshot; without it (a segment that was lost) they are dropped.
  private synced = false;

  read(r: Rec): Rec[] {
    const snapshot = r.type === FULL_SNAPSHOT && r.format === CHANGE_FORMAT && Array.isArray(r.data);
    if (!snapshot && r.type !== CHANGE) return [r];
    if (snapshot) this.reset();
    if (!this.synced || !Array.isArray(r.data)) return [];
    try {
      return this.changes(r, r.data as Item[], snapshot);
    } catch (e) {
      if (!(e instanceof UnknownString)) throw e;
      this.synced = false;
      return [];
    }
  }

  private reset() {
    this.strings = [];
    this.nextNode = 0;
    this.sheets = [];
    this.parentOf.clear();
    this.childrenOf.clear();
    this.shownIn.clear();
    this.names.clear();
    this.adopted.clear();
    this.synced = true;
  }

  private changes(r: Rec, changes: Item[], snapshot: boolean): Rec[] {
    const out: Rec[] = [];
    const tree = new Map<number, SNode>();
    let root: SNode | undefined;
    const emit = (data: Record<string, unknown>) => out.push({ type: INCREMENTAL, timestamp: r.timestamp, data });
    const mutation = (m: Record<string, unknown>) => emit({ source: 0, adds: [], removes: [], texts: [], attributes: [], ...m });
    for (const change of changes) {
      const items = change.slice(1) as Item[];
      switch (change[0]) {
        case Change.AddString:
          for (const s of items) this.strings.push(String(s));
          break;
        case Change.AddStrings:
          for (const [, ...strs] of items) for (const s of strs) this.strings.push(String(s));
          break;
        case Change.ClearStrings:
          this.strings = [];
          break;
        case Change.AddNode: {
          const adds = items.map((item) => this.addNode(item));
          if (!snapshot) {
            mutation({ adds });
            break;
          }
          for (const a of adds) {
            if (a.node.type === 11) continue; // its children go to the host
            tree.set(a.node.id, a.node);
            const parent = tree.get(a.parentId);
            if (!parent) {
              root ??= a.node;
              continue;
            }
            const siblings = (parent.childNodes ??= []);
            const at = a.nextId == null ? -1 : siblings.findIndex((n) => n.id === a.nextId);
            siblings.splice(at < 0 ? siblings.length : at, 0, a.node);
          }
          break;
        }
        case Change.RemoveNode:
          for (const id of items as unknown as number[]) this.forget(id);
          mutation({ removes: (items as unknown as number[]).map((id) => ({ id })) });
          break;
        case Change.Attribute:
          mutation({
            attributes: items.map(([id, ...assignments]) => ({
              id,
              attributes: Object.fromEntries((assignments as Str[][]).map(([k, v]) => [this.str(k), v === undefined ? null : this.str(v)])),
            })),
          });
          break;
        case Change.Text:
          mutation({ texts: items.map(([id, value]) => ({ id, value: this.str(value as Str) })) });
          break;
        case Change.ScrollPosition:
          for (const [id, x, y] of items) emit({ source: 3, id, x, y });
          break;
        case Change.InputValue:
          for (const [id, value] of items) emit({ source: 5, id, text: this.str(value as Str) });
          break;
        case Change.InputSelection:
          for (const [state, ...ids] of items) for (const id of ids) emit({ source: 5, id, isChecked: state === 0 });
          break;
        case Change.AddStyleSheet:
          for (const sheet of items) this.sheets.push(this.css(sheet));
          break;
        case Change.AttachedStyleSheets:
          for (const [id, ...sheetIds] of items as number[][]) {
            const css = sheetIds.map((s) => this.sheets[s] ?? '').join('\n');
            const name = this.names.get(id);
            if (name && !name.startsWith('#')) {
              mutation({ attributes: [{ id, attributes: { _cssText: css } }] });
              continue;
            }
            // A document or shadow root adopted them: a <style> in the document's head, or in the shadow host.
            const parentId = name === '#document' ? this.head() : this.shownIn.get(id);
            if (parentId === undefined) continue;
            const previous = this.adopted.get(id);
            if (previous !== undefined) mutation({ removes: [{ id: previous }] });
            const styleId = this.nextSynthetic--;
            this.adopted.set(id, styleId);
            mutation({ adds: [{ parentId, nextId: null, node: { type: 2, id: styleId, tagName: 'style', attributes: { _cssText: css }, childNodes: [] } }] });
          }
          break;
      }
    }
    if (snapshot) out.unshift({ type: FULL_SNAPSHOT, timestamp: r.timestamp, data: { node: root ?? { type: 0, id: 0, childNodes: [] } } });
    return out;
  }

  private str(v: Str): string {
    if (typeof v === 'string') return v;
    if (typeof v === 'object' && v !== null) return String(v.string);
    const s = this.strings[v];
    if (s === undefined) throw new UnknownString(String(v));
    return s;
  }

  private css(sheet: unknown): string {
    const [rules, media, disabled] = sheet as [Str | Str[], Str[]?, boolean?];
    if (disabled) return '';
    const text = Array.isArray(rules) ? rules.map((s) => this.str(s)).join('\n') : this.str(rules);
    return media?.length ? `@media ${media.map((m) => this.str(m)).join(', ')} {\n${text}\n}` : text;
  }

  /** The id of the document's <head>, where adopted stylesheets go. */
  private head(): number | undefined {
    for (const [id, name] of this.names) if (name === 'HEAD') return id;
    return undefined;
  }

  /** Where a new node goes (from its insertion point) and what it is; updates the node tree the ids refer to. */
  private addNode(item: Item): Add {
    const id = this.nextNode++;
    const [at, rawName, ...rest] = item as [number | null, Str, ...unknown[]];
    const name = this.str(rawName);
    let parent: number | undefined;
    let next: number | null = null;
    if (at === null) parent = undefined;
    else if (at === 0) {
      // After the node added just before it.
      parent = this.parentOf.get(id - 1);
      const siblings = parent === undefined ? undefined : this.childrenOf.get(parent);
      const i = siblings?.indexOf(id - 1) ?? -1;
      next = i >= 0 ? (siblings![i + 1] ?? null) : null;
    } else if (at < 0) {
      next = id + at;
      parent = this.parentOf.get(next);
    } else parent = this.shownIn.get(id - at) ?? id - at;
    this.names.set(id, name);
    const node = this.node(id, name, rest);
    if (parent !== undefined) {
      this.parentOf.set(id, parent);
      if (name === '#shadow-root' || name === '#document-fragment') this.shownIn.set(id, parent);
      else {
        const siblings = this.childrenOf.get(parent) ?? [];
        const i = next === null ? -1 : siblings.indexOf(next);
        siblings.splice(i < 0 ? siblings.length : i, 0, id);
        this.childrenOf.set(parent, siblings);
      }
    }
    return { parentId: parent ?? -1, nextId: next, node };
  }

  private node(id: number, name: string, rest: unknown[]): SNode {
    switch (name) {
      case '#document':
        return { type: 0, id, childNodes: [] };
      case '#doctype':
        return { type: 1, id, name: this.str(rest[0] as Str) };
      case '#text':
        return { type: 3, id, textContent: this.str(rest[0] as Str) };
      case '#cdata-section':
        return { type: 4, id, textContent: '' };
      case '#shadow-root':
      case '#document-fragment':
        return { type: 11, id, childNodes: [] };
    }
    const svg = name.startsWith('svg>');
    const attributes = Object.fromEntries((rest as Str[][]).map(([k, v]) => [this.str(k), this.str(v)]));
    return { type: 2, id, tagName: svg ? name.slice(4) : name.toLowerCase(), attributes, childNodes: [], ...(svg ? { isSVG: true } : {}) };
  }

  /** Drops a removed node and what it contained from the tree. */
  private forget(id: number) {
    const parent = this.parentOf.get(id);
    const siblings = parent === undefined ? undefined : this.childrenOf.get(parent);
    const i = siblings?.indexOf(id) ?? -1;
    if (i >= 0) siblings!.splice(i, 1);
    const stack = [id];
    while (stack.length) {
      const n = stack.pop()!;
      stack.push(...(this.childrenOf.get(n) ?? []));
      this.childrenOf.delete(n);
      this.parentOf.delete(n);
      this.names.delete(n);
      this.shownIn.delete(n);
    }
  }
}
