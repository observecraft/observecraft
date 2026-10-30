// Squarified treemap layout (Bruls, Huizing, van Wijk): boxes close to squares, biggest first.

export interface Box<T> {
  item: T;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Lays `items` (by `value`) out in a `width` × `height` rectangle. */
export function squarify<T>(items: T[], value: (t: T) => number, width: number, height: number): Box<T>[] {
  const total = items.reduce((n, t) => n + Math.max(0, value(t)), 0);
  if (!total || width <= 0 || height <= 0) return [];
  const scale = (width * height) / total;
  const queue = [...items].filter((t) => value(t) > 0).sort((a, b) => value(b) - value(a));
  const out: Box<T>[] = [];
  let rect = { x: 0, y: 0, w: width, h: height };
  // The worst aspect ratio of a row of areas laid along a side of `side` length.
  const worst = (row: number[], side: number) => {
    const sum = row.reduce((a, b) => a + b, 0);
    const max = Math.max(...row);
    const min = Math.min(...row);
    return Math.max((side * side * max) / (sum * sum), (sum * sum) / (side * side * min));
  };
  while (queue.length) {
    const side = Math.min(rect.w, rect.h);
    const row: T[] = [queue.shift()!];
    while (queue.length) {
      const areas = row.map((t) => value(t) * scale);
      const next = value(queue[0]) * scale;
      if (worst([...areas, next], side) > worst(areas, side)) break;
      row.push(queue.shift()!);
    }
    const sum = row.reduce((n, t) => n + value(t) * scale, 0);
    const thickness = sum / side;
    let offset = 0;
    for (const t of row) {
      const length = (value(t) * scale) / thickness;
      out.push(rect.w >= rect.h ? { item: t, x: rect.x, y: rect.y + offset, w: thickness, h: length } : { item: t, x: rect.x + offset, y: rect.y, w: length, h: thickness });
      offset += length;
    }
    rect = rect.w >= rect.h ? { x: rect.x + thickness, y: rect.y, w: rect.w - thickness, h: rect.h } : { x: rect.x, y: rect.y + thickness, w: rect.w, h: rect.h - thickness };
  }
  return out;
}
