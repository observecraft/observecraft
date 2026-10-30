import type { CSSProperties } from 'react';

export interface ConditionalFormat {
  comparator: string;
  value: number;
  palette: string;
}

/** The first format whose condition `value` meets, as the upstream widgets apply them. */
export function matchFormat(value: number | undefined, formats: ConditionalFormat[] | undefined): ConditionalFormat | undefined {
  if (value === undefined || Number.isNaN(value)) return undefined;
  return formats?.find((f) => ({ '>': value > f.value, '>=': value >= f.value, '<': value < f.value, '<=': value <= f.value, '=': value === f.value, '==': value === f.value })[f.comparator]);
}

/** Colors of integrations-core's palette names (`white_on_red`, `black_on_light_yellow`, `green_on_white`, …). */
export function formatStyle(palette: string): CSSProperties | undefined {
  const m = /^(white|black|red|yellow|green|gray)_on_(light_)?(red|yellow|green|gray|white)$/.exec(palette);
  if (!m) return undefined;
  const [, text, light, background] = m;
  if (background === 'white') return { color: `var(--cf-${text})` };
  return { background: `var(--cf-${light ? 'light-' : ''}${background})`, color: text === 'white' ? 'var(--text-inverse)' : 'var(--text)' };
}

/** Whether the palette fills its cell or card (rather than only coloring the text). */
export const fills = (palette: string) => !/_on_white$/.test(palette) && formatStyle(palette) !== undefined;
