// Presentation conversions only. Query/formula evaluation always uses the original values.

/** Base unit and the factor to it, for unit names as OTel (UCUM) and integrations-core's metadata.csv write them. */
const BASES: Record<string, [string, number]> = {
  'n{core}': ['core', 1e-9],
  nanocore: ['core', 1e-9],
  millicore: ['core', 1e-3],
  'm{core}': ['core', 1e-3],
  '{core}': ['core', 1],
  core: ['core', 1],
  // kubernetes_state's metadata.csv calls cores "cpu".
  cpu: ['core', 1],
  byte: ['By', 1],
  bytes: ['By', 1],
  By: ['By', 1],
  kibibyte: ['By', 1024],
  KiBy: ['By', 1024],
  mebibyte: ['By', 1024 ** 2],
  MiBy: ['By', 1024 ** 2],
  MiB: ['By', 1024 ** 2],
  gibibyte: ['By', 1024 ** 3],
  GiBy: ['By', 1024 ** 3],
  GiB: ['By', 1024 ** 3],
  percent: ['%', 1],
  '%': ['%', 1],
  fraction: ['%', 100],
  nanosecond: ['s', 1e-9],
  ns: ['s', 1e-9],
  microsecond: ['s', 1e-6],
  us: ['s', 1e-6],
  millisecond: ['s', 1e-3],
  ms: ['s', 1e-3],
  second: ['s', 1],
  s: ['s', 1],
  minute: ['s', 60],
  hour: ['s', 3600],
  request: ['req', 1],
  operation: ['op', 1],
};

/** Per-unit suffixes ("byte/second" → "/s"). */
const PER: Record<string, string> = { second: '/s', s: '/s', minute: '/min', hour: '/h' };

export function presentationUnit(unit?: string): { unit: string; scale: number } {
  if (!unit) return { unit: '', scale: 1 };
  const [name, per] = unit.split('/', 2);
  const [base, scale] = BASES[name] ?? [name, 1];
  return { unit: per === undefined ? base : `${base}${PER[per] ?? `/${per}`}`, scale };
}

const number = (value: number, precision: number) => value.toLocaleString('en-US', { maximumFractionDigits: precision });

/** 1234567 → "1.23M": counts and unitless values. */
function compact(value: number, precision: number): string {
  const abs = Math.abs(value);
  for (const [limit, suffix] of [
    [1e12, 'T'],
    [1e9, 'B'],
    [1e6, 'M'],
    [1e3, 'k'],
  ] as const) {
    if (abs >= limit) return `${number(value / limit, precision)}${suffix}`;
  }
  return number(value, precision);
}

/** Formats a value already expressed in the presentation unit. */
export function formatUnitValue(value: number, unit: string, precision = 2): string {
  const [base, per] = [unit.split('/')[0], unit.includes('/') ? `/${unit.split('/').slice(1).join('/')}` : ''];
  if (base === 'By') {
    const index = value ? Math.min(4, Math.max(0, Math.floor(Math.log(Math.abs(value)) / Math.log(1024)))) : 0;
    return `${number(value / 1024 ** index, precision)} ${['B', 'KiB', 'MiB', 'GiB', 'TiB'][index]}${per}`;
  }
  if (base === 'core' && !per) return value !== 0 && Math.abs(value) < 1 ? `${number(value * 1000, precision)} mCPU` : `${number(value, precision)} ${value === 1 ? 'core' : 'cores'}`;
  if (base === '%') return `${number(value, precision)}%${per}`;
  if (base === 's' && !per) {
    const abs = Math.abs(value);
    if (abs !== 0 && abs < 1e-6) return `${number(value * 1e9, precision)} ns`;
    if (abs !== 0 && abs < 1e-3) return `${number(value * 1e6, precision)} µs`;
    if (abs !== 0 && abs < 1) return `${number(value * 1e3, precision)} ms`;
    return `${number(value, precision)} s`;
  }
  if (base === '1' || !base) return `${compact(value, precision)}${per}`;
  const word = base.replace(/[{}]/g, '');
  // Count units read as words: "14 nodes", "1 node"; abbreviations stay as they are.
  const plural = value !== 1 && /^[a-z]+$/.test(word) && !word.endsWith('s') && !ABBREVIATIONS.has(word);
  return `${compact(value, precision)} ${plural ? `${word}s` : word}${per}`;
}

const ABBREVIATIONS = new Set(['req', 'op']);

/** Tick step for a byte axis up to `max` bytes: a round number of B/KiB/MiB/GiB/TiB, about five ticks. */
export function byteAxisInterval(max: number, ticks = 5): number | undefined {
  if (!(max > 0)) return undefined;
  const step = max / ticks;
  const unit = 1024 ** Math.max(0, Math.floor(Math.log(step) / Math.log(1024)));
  const nice = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000].find((n) => n >= step / unit) ?? 1000;
  return nice * unit;
}

export function formatMetricValue(value: number, unit?: string, precision = 2): string {
  const display = presentationUnit(unit);
  return formatUnitValue(value * display.scale, display.unit, precision);
}
