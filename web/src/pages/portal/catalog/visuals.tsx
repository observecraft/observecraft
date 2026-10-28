// Software Catalog marks: section and kind icons, the entity icon (language or technology logo),
// view tab icons and small number formats.
import { AppWindow, Boxes, Braces, CircleUser, Coins, Database, Gauge, GitBranch, Globe, Hexagon, Layers, Lock, Rocket, Route, ScanSearch, ShieldCheck, Users, type LucideIcon } from 'lucide-react';
import type { PortalEntity, PortalEntityKind } from '../../../api/portalCatalog/types';
import { BrandLogo, hasBrandLogo } from '../../../components/brand/BrandLogo';
import { LANGUAGE_LABEL, serviceLogo } from '../../apm/visuals';
import type { Section, ViewId } from './catalogModel';

export const SECTION_ICON: Record<Section, LucideIcon> = {
  all: Boxes,
  mine: CircleUser,
  inferred: ScanSearch,
  service: Hexagon,
  datastore: Database,
  queue: Layers,
  api: Braces,
  system: Boxes,
  frontend: AppWindow,
  endpoint: Route,
  external_provider: Globe,
  repository: GitBranch,
};

export const VIEW_ICON: Record<ViewId, LucideIcon> = { ownership: Users, reliability: ShieldCheck, performance: Gauge, security: Lock, costs: Coins, delivery: Rocket };

// Languages users write in definitions → logo keys (telemetry languages go through serviceLogo).
const DEFINED_LANGUAGE_LOGO: Record<string, string> = { javascript: 'javascript', typescript: 'typescript', rust: 'rust', ruby: 'ruby', php: 'php', kotlin: 'kotlin' };

/** The entity's logo (language, datastore or queue technology) or its kind icon. */
export function EntityIcon({ entity, size = 16 }: { entity: Pick<PortalEntity, 'kind' | 'name' | 'languages' | 'technology' | 'entityType'>; size?: number }) {
  const language = entity.languages[0] ?? null;
  const defined = language ? DEFINED_LANGUAGE_LOGO[language] : undefined;
  const logo =
    serviceLogo(entity.name, language, entity.technology ?? (entity.kind === 'datastore' || entity.kind === 'queue' ? entity.entityType : null)) ?? (defined && hasBrandLogo(defined) ? defined : null);
  if (logo) return <BrandLogo name={logo} size={size} title={language ? (LANGUAGE_LABEL[language] ?? language) : (entity.technology ?? entity.name)} />;
  const Icon = SECTION_ICON[entity.kind as PortalEntityKind];
  return <Icon size={size} className="pc-kind-icon" aria-hidden />;
}

export const fmtPercent = (frac: number | null | undefined, digits = 1) => (frac === null || frac === undefined ? '—' : `${(frac * 100).toFixed(frac > 0 && frac < 0.001 ? 2 : digits)}%`);
export const fmtNumber = (v: number | null | undefined) => (v === null || v === undefined ? '—' : v >= 100 ? Math.round(v).toLocaleString() : v >= 10 ? v.toFixed(1) : v.toFixed(2));
