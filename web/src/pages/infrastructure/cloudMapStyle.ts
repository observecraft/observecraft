// Colours of the Cloudcraft diagram: face shading for the isometric boxes, per resource type, group
// level and health. Base colours come from the theme tokens (read once per render).
import type { CatalogType } from '../../api/catalog/client';
import type { CloudMapHealth, CloudMapOverlay } from '../../api/cloudMap/client';

export interface Palette {
  brand: string;
  blue: string;
  purple: string;
  orange: string;
  green: string;
  gray: string;
  ok: string;
  warn: string;
  error: string;
  nodata: string;
  text: string;
  muted: string;
  border: string;
  bg: string;
}

export const DEFAULT_PALETTE: Palette = {
  brand: '#0b5f5c',
  blue: '#1d6fd8',
  purple: '#7a45d9',
  orange: '#e98404',
  green: '#2ea86b',
  gray: '#5a6273',
  ok: '#2ea86b',
  warn: '#f2a93b',
  error: '#e5383b',
  nodata: '#c8ccd6',
  text: '#1f2430',
  muted: '#5a6273',
  border: '#d7dbe3',
  bg: '#ffffff',
};

function rgb(hex: string): [number, number, number] {
  const h = hex.replace('#', '');
  const full = h.length === 3 ? [...h].map((c) => c + c).join('') : h.padEnd(6, '0').slice(0, 6);
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16) || 0) as [number, number, number];
}

/** `a` moved towards `b` by t (0 – 1), as #rrggbb. */
export function mix(a: string, b: string, t: number): string {
  const x = rgb(a);
  const y = rgb(b);
  return `#${x
    .map((c, i) =>
      Math.round(c + (y[i] - c) * t)
        .toString(16)
        .padStart(2, '0'),
    )
    .join('')}`;
}

/** Lit top, shaded front-left and mid front-right faces of a box of `base` colour. */
export function faces(base: string): { top: string; left: string; right: string; edge: string } {
  return { top: mix(base, '#ffffff', 0.2), left: mix(base, '#000000', 0.22), right: mix(base, '#000000', 0.08), edge: mix(base, '#000000', 0.35) };
}

const CLOUD_COLORS: Record<string, keyof Palette> = { aws: 'orange', azure: 'blue', gcp: 'green', oracle: 'error', alibaba: 'orange' };

/** A block's colour: by health on the health overlay, else by resource type (hosts by their cloud). */
export function blockColor(p: Palette, type: CatalogType, cloud: string | null | undefined, health: CloudMapHealth, overlay: CloudMapOverlay): string {
  if (overlay === 'health') return health === 'error' ? p.error : health === 'warning' ? p.warn : health === 'ok' ? p.ok : p.nodata;
  if (type === 'host') return p[CLOUD_COLORS[cloud ?? ''] ?? 'gray'];
  if (type === 'kube_node') return p.gray;
  if (type === 'kube_service' || type === 'ecs_service') return p.purple;
  if (type === 'kube_ingress') return p.orange;
  if (type === 'kube_cronjob') return p.blue;
  if (type === 'ecs_task') return p.orange;
  return p.brand;
}

/** A group platform's fill: a light tint per group key, a little deeper at each level. */
export function groupColor(p: Palette, key: string, level: number): { top: string; side: string; stroke: string; label: string } {
  const base: Record<string, string> = {
    cloud: p.gray,
    account: p.blue,
    region: p.blue,
    availability_zone: p.purple,
    cluster: p.brand,
    namespace: p.green,
    env: p.orange,
    service: p.purple,
    team: p.orange,
  };
  const c = base[key] ?? p.gray;
  const top = mix(c, '#ffffff', Math.max(0.78, 0.93 - level * 0.05));
  return { top, side: mix(c, '#ffffff', 0.55), stroke: mix(c, '#ffffff', 0.35), label: mix(c, '#000000', 0.25) };
}
