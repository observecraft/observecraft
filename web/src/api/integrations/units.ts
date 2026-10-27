// Presentation conversions only. Query/formula evaluation always uses the original values.
export function presentationUnit(unit?: string): { unit: string; scale: number } {
  switch (unit) {
    case 'n{core}':
    case 'nanocore':
      return { unit: 'core', scale: 1e-9 };
    case 'millicore':
    case 'm{core}':
      return { unit: 'core', scale: 1e-3 };
    case '{core}':
    case 'core':
      return { unit: 'core', scale: 1 };
    case 'byte':
    case 'bytes':
      return { unit: 'By', scale: 1 };
    case 'GiBy':
    case 'GiB':
      return { unit: 'By', scale: 1024 ** 3 };
    case 'MiBy':
    case 'MiB':
      return { unit: 'By', scale: 1024 ** 2 };
    default:
      return { unit: unit ?? '', scale: 1 };
  }
}

const number = (value: number, precision: number) => value.toLocaleString('en-US', { maximumFractionDigits: precision });

/** Formats a value already expressed in the presentation unit. */
export function formatUnitValue(value: number, unit: string, precision = 2): string {
  if (unit === 'By' || unit === 'By/s') {
    const suffix = unit.endsWith('/s') ? '/s' : '';
    const index = value ? Math.min(4, Math.max(0, Math.floor(Math.log(Math.abs(value)) / Math.log(1024)))) : 0;
    return `${number(value / 1024 ** index, precision)} ${['B', 'KiB', 'MiB', 'GiB', 'TiB'][index]}${suffix}`;
  }
  if (unit === 'core') return value !== 0 && Math.abs(value) < 1 ? `${number(value * 1000, precision)} mCPU` : `${number(value, precision)} cores`;
  if (unit === '%') return `${number(value, precision)}%`;
  if (unit === '1' || !unit) return number(value, precision);
  return `${number(value, precision)} ${unit.replace(/[{}]/g, '')}`;
}

export function formatMetricValue(value: number, unit?: string, precision = 2): string {
  const display = presentationUnit(unit);
  return formatUnitValue(value * display.scale, display.unit, precision);
}
