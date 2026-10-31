/** 3_400_000_000 → "3.4B", 232_320_000 → "232.32M", 1290 → "1.29k". */
export function big(n: number): string {
  const v = Math.abs(n);
  const [d, s] = v >= 1e9 ? [1e9, 'B'] : v >= 1e6 ? [1e6, 'M'] : v >= 1e3 ? [1e3, 'k'] : [1, ''];
  if (d === 1) return String(Math.round(n));
  const x = n / d;
  return `${+x.toFixed(Math.abs(x) >= 100 ? 0 : Math.abs(x) >= 10 ? 1 : 2)}${s}`;
}

/** Decimal byte units, as ingestion volumes are counted: 322_000 → "322 KB". */
export function bytes(n: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let v = Math.abs(n);
  let u = 0;
  while (v >= 1000 && u < units.length - 1) {
    v /= 1000;
    u++;
  }
  return `${v >= 100 || u === 0 ? Math.round(v) : +v.toFixed(v >= 10 ? 1 : 2)} ${units[u]}`;
}

/** Fraction → "81.5%". */
export function share(frac: number): string {
  const p = frac * 100;
  return `${p >= 99.95 ? 100 : p < 0.1 && p > 0 ? '<0.1' : p.toFixed(1)}%`;
}
