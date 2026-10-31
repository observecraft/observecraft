import { useCallback } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Code2 } from 'lucide-react';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';

/** The application types of New Application: the SDK, and the API type it creates. */
export interface AppKind {
  id: string;
  label: string;
  /** The application's `type`. */
  type: string;
  logo?: string;
  badge?: 'NEW' | 'PREVIEW';
  /** Not available yet in ObserveCraft. */
  soon?: boolean;
}

export const APP_KINDS: AppKind[] = [
  { id: 'js', label: 'JS', type: 'browser', logo: 'javascript' },
  { id: 'react', label: 'React', type: 'browser', logo: 'react' },
  { id: 'nextjs', label: 'Next.js', type: 'browser', logo: 'nextdotjs', badge: 'NEW' },
  { id: 'angular', label: 'Angular', type: 'browser', logo: 'angular', badge: 'NEW' },
  { id: 'vue', label: 'Vue.js', type: 'browser', logo: 'vuedotjs', badge: 'NEW' },
  { id: 'nuxt', label: 'Nuxt', type: 'browser', logo: 'nuxt', badge: 'NEW' },
  { id: 'salesforce', label: 'Salesforce', type: 'browser', logo: 'salesforce', badge: 'PREVIEW' },
  { id: 'shopify', label: 'Shopify', type: 'browser', logo: 'shopify', badge: 'PREVIEW' },
  { id: 'android', label: 'Android', type: 'android', logo: 'android' },
  { id: 'ios', label: 'iOS', type: 'ios', logo: 'apple' },
  { id: 'react-native', label: 'React Native', type: 'react-native', logo: 'react' },
  { id: 'flutter', label: 'Flutter', type: 'flutter', logo: 'flutter' },
  { id: 'roku', label: 'Roku', type: 'roku' },
  { id: 'electron', label: 'Electron', type: 'electron', logo: 'electron', badge: 'PREVIEW' },
  { id: 'unity', label: 'Unity', type: 'unity', logo: 'unity' },
  { id: 'cpp', label: 'C++', type: 'browser', logo: 'cplusplus', badge: 'PREVIEW', soon: true },
  { id: 'kotlin', label: 'Kotlin Multiplatform', type: 'kotlin-multiplatform', logo: 'kotlin' },
  { id: 'maui', label: '.NET MAUI', type: 'browser', logo: 'dotnet', badge: 'PREVIEW', soon: true },
];

export const TYPE_LABEL: Record<string, string> = {
  browser: 'Browser',
  ios: 'iOS',
  android: 'Android',
  'react-native': 'React Native',
  flutter: 'Flutter',
  roku: 'Roku',
  electron: 'Electron',
  unity: 'Unity',
  'kotlin-multiplatform': 'Kotlin Multiplatform',
};

const TYPE_LOGO: Record<string, string> = {
  browser: 'javascript',
  ios: 'apple',
  android: 'android',
  'react-native': 'react',
  flutter: 'flutter',
  electron: 'electron',
  unity: 'unity',
  'kotlin-multiplatform': 'kotlin',
};

/** An application's type as its logo (JS for browser applications). */
export function AppTypeIcon({ type, size = 20 }: { type: string; size?: number }) {
  const logo = TYPE_LOGO[type];
  if (logo && hasBrandLogo(logo)) return <BrandLogo name={logo} size={size} title={TYPE_LABEL[type]} />;
  return <Code2 size={size} aria-label={TYPE_LABEL[type] ?? type} />;
}

export const isMobile = (type: string) => !['browser', 'electron'].includes(type);

const STORAGE_KEY = 'oc.rum.application';

/** The application RUM pages show (`?application=`), remembered per viewer. */
export function useSelectedApplication(): [string | null, (id: string | null) => void] {
  const [params, setParams] = useSearchParams();
  let stored: string | null = null;
  try {
    stored = localStorage.getItem(STORAGE_KEY);
  } catch {
    /* storage unavailable */
  }
  const id = params.get('application') ?? stored;
  const set = useCallback(
    (next: string | null) => {
      try {
        if (next) localStorage.setItem(STORAGE_KEY, next);
        else localStorage.removeItem(STORAGE_KEY);
      } catch {
        /* storage unavailable */
      }
      setParams(
        (p) => {
          const n = new URLSearchParams(p);
          if (next) n.set('application', next);
          else n.delete('application');
          return n;
        },
        { replace: true },
      );
    },
    [setParams],
  );
  return [id || null, set];
}

/** Nanoseconds (RUM timings) as 1.2s / 420ms. */
export function fmtNs(ns: string | number | undefined): string {
  const v = Number(ns);
  if (!Number.isFinite(v) || ns === undefined || ns === '') return '—';
  return fmtMsShort(v / 1e6);
}

export function fmtMsShort(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return '—';
  if (ms >= 60_000) return `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
  if (ms >= 1000) return `${(ms / 1000).toFixed(ms >= 10_000 ? 1 : 2).replace(/\.?0+$/, '')}s`;
  return `${Math.round(ms)}ms`;
}
