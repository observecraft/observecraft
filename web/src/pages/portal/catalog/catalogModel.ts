// Software Catalog page model: navigation sections and counts, view columns, filters and the
// entity form ↔ definition mapping. Pure, tested in scripts/portal-catalog.test.mjs.
import type { EntityDefinition, PortalEntity, PortalEntityKind } from '../../../api/portalCatalog/types';

export type Section = 'all' | 'mine' | 'inferred' | PortalEntityKind;
export type ViewId = 'ownership' | 'reliability' | 'performance' | 'security' | 'costs' | 'delivery';

export const KIND_LABEL: Record<PortalEntityKind, string> = {
  service: 'Service',
  datastore: 'Datastore',
  queue: 'Queue',
  api: 'API',
  system: 'System',
  frontend: 'Frontend App',
  endpoint: 'Endpoint',
  external_provider: 'External Provider',
  repository: 'Repository',
};

/** The left navigation's software sections, in order. */
export const SOFTWARE_SECTIONS: { id: Section; label: string }[] = [
  { id: 'mine', label: 'My Entities' },
  { id: 'system', label: 'Systems' },
  { id: 'service', label: 'Services' },
  { id: 'queue', label: 'Queues' },
  { id: 'datastore', label: 'Datastores' },
  { id: 'inferred', label: 'Inferred Services' },
  { id: 'external_provider', label: 'External Providers' },
  { id: 'frontend', label: 'Frontend Apps' },
  { id: 'repository', label: 'Repositories' },
  { id: 'api', label: 'APIs' },
  { id: 'endpoint', label: 'Endpoints' },
];

export const VIEWS: { id: ViewId; label: string }[] = [
  { id: 'ownership', label: 'Ownership' },
  { id: 'reliability', label: 'Reliability' },
  { id: 'performance', label: 'Performance' },
  { id: 'security', label: 'Security' },
  { id: 'costs', label: 'Costs' },
  { id: 'delivery', label: 'Delivery' },
];

export type Column =
  | 'name'
  | 'type'
  | 'team'
  | 'onCall'
  | 'contacts'
  | 'repo'
  | 'docs'
  | 'slos'
  | 'monitors'
  | 'incidents'
  | 'errors'
  | 'requests'
  | 'errorRate'
  | 'p95'
  | 'vulnerabilities'
  | 'signals'
  | 'cost'
  | 'costChange'
  | 'version'
  | 'lastDeploy'
  | 'ciSuccess'
  | 'lastCiRun';

const COLUMNS: Record<ViewId, Column[]> = {
  ownership: ['name', 'type', 'team', 'onCall', 'contacts', 'repo', 'docs'],
  reliability: ['name', 'team', 'slos', 'monitors', 'incidents', 'errors'],
  performance: ['name', 'team', 'requests', 'errorRate', 'p95'],
  security: ['name', 'team', 'vulnerabilities', 'signals'],
  costs: ['name', 'team', 'cost', 'costChange'],
  delivery: ['name', 'team', 'version', 'lastDeploy', 'ciSuccess', 'lastCiRun'],
};

export const columnsFor = (view: ViewId): Column[] => COLUMNS[view];

export const COLUMN_LABEL: Record<Column, string> = {
  name: 'Name',
  type: 'Type',
  team: 'Team',
  onCall: 'On-Call',
  contacts: 'Contacts',
  repo: 'Repository',
  docs: 'Docs',
  slos: 'SLOs',
  monitors: 'Monitors in alert',
  incidents: 'Incidents',
  errors: 'Error types',
  requests: 'Requests/s',
  errorRate: 'Error rate',
  p95: 'P95 latency',
  vulnerabilities: 'Vulnerabilities',
  signals: 'Security signals',
  cost: 'Cost (30d)',
  costChange: 'Change',
  version: 'Version',
  lastDeploy: 'Last deploy',
  ciSuccess: 'CI success',
  lastCiRun: 'Last CI run',
};

export interface NavCounts {
  software: number;
  mine: number;
  inferred: number;
  [kind: string]: number;
}

export function navCounts(entities: Pick<PortalEntity, 'kind' | 'owner' | 'inferred'>[], myTeams: string[]): NavCounts {
  const out: NavCounts = { software: entities.length, mine: 0, inferred: 0 };
  for (const e of entities) {
    out[e.kind] = (out[e.kind] ?? 0) + 1;
    if (e.inferred) out.inferred++;
    if (e.owner && myTeams.includes(e.owner)) out.mine++;
  }
  return out;
}

export interface Filter {
  section: Section;
  /** Words matching the name, display name or tags. */
  search: string;
  teams: string[];
  myTeams: string[];
}

export function filterEntities<T extends Pick<PortalEntity, 'kind' | 'name' | 'owner' | 'tags' | 'displayName' | 'inferred'>>(entities: T[], f: Filter): T[] {
  const words = f.search.trim().toLowerCase().split(/\s+/).filter(Boolean);
  return entities.filter((e) => {
    if (f.section === 'mine' && !(e.owner && f.myTeams.includes(e.owner))) return false;
    if (f.section === 'inferred' && !e.inferred) return false;
    if (f.section !== 'all' && f.section !== 'mine' && f.section !== 'inferred' && e.kind !== f.section) return false;
    if (f.teams.length && !f.teams.includes(e.owner)) return false;
    const hay = [e.name, e.displayName ?? '', ...e.tags].join(' ').toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

/** Where an entity's own product page is, if it has one. */
export function productLink(e: Pick<PortalEntity, 'kind' | 'name' | 'service' | 'sources'>): string | null {
  if (e.kind === 'service' && e.sources.includes('apm')) return `/apm/services/${encodeURIComponent(e.name)}`;
  if (e.kind === 'endpoint' && e.service) return `/apm/services/${encodeURIComponent(e.service)}?resource=${encodeURIComponent(e.name)}`;
  return null;
}

// ================================================================ entity form

export interface EntityForm {
  kind: string;
  name: string;
  displayName: string;
  owner: string;
  system: string;
  description: string;
  lifecycle: string;
  tier: string;
  type: string;
  languages: string;
  tags: string;
  dependsOn: string;
  links: { name: string; type: string; url: string }[];
  contacts: { name: string; type: string; contact: string }[];
}

export const emptyForm = (kind = 'service'): EntityForm => ({
  kind,
  name: '',
  displayName: '',
  owner: '',
  system: '',
  description: '',
  lifecycle: '',
  tier: '',
  type: '',
  languages: '',
  tags: '',
  dependsOn: '',
  links: [],
  contacts: [],
});

const csv = (s: string) =>
  s
    .split(',')
    .map((x) => x.trim())
    .filter(Boolean);
const opt = (s: string) => s.trim() || null;

/**
 * The definition a form describes (the server and the mock validate it). Fields the form doesn't
 * edit (system components, API implementations, integrations, …) are kept from `base`.
 */
export function definitionFromForm(f: EntityForm, base?: EntityDefinition): EntityDefinition {
  return {
    ...base,
    apiVersion: 'v3',
    kind: f.kind,
    metadata: {
      ...base?.metadata,
      name: f.name.trim(),
      displayName: opt(f.displayName),
      owner: opt(f.owner),
      description: opt(f.description),
      tags: csv(f.tags),
      links: f.links.filter((l) => l.url.trim()).map((l) => ({ name: l.name.trim() || l.url.trim(), type: l.type, url: l.url.trim() })),
      contacts: f.contacts.filter((c) => c.contact.trim()).map((c) => ({ ...(c.name.trim() ? { name: c.name.trim() } : {}), type: c.type, contact: c.contact.trim() })),
    },
    spec: {
      ...base?.spec,
      lifecycle: opt(f.lifecycle),
      tier: opt(f.tier),
      type: opt(f.type),
      languages: csv(f.languages),
      dependsOn: csv(f.dependsOn),
      componentOf: csv(f.system),
    },
  };
}

export function formFromDefinition(d: EntityDefinition): EntityForm {
  const systems = (d.spec?.componentOf ?? []).map((r) => r.replace(/^system:/, ''));
  return {
    kind: d.kind,
    name: d.metadata.name,
    displayName: d.metadata.displayName ?? '',
    owner: d.metadata.owner ?? '',
    system: systems.join(', '),
    description: d.metadata.description ?? '',
    lifecycle: d.spec?.lifecycle ?? '',
    tier: d.spec?.tier ?? '',
    type: d.spec?.type ?? '',
    languages: (d.spec?.languages ?? []).join(', '),
    tags: (d.metadata.tags ?? []).join(', '),
    dependsOn: (d.spec?.dependsOn ?? []).join(', '),
    links: (d.metadata.links ?? []).map((l) => ({ name: l.name, type: l.type, url: l.url })),
    contacts: (d.metadata.contacts ?? []).map((c) => ({ name: c.name ?? '', type: c.type, contact: c.contact })),
  };
}

const isBlank = (v: unknown): boolean => v === null || v === undefined || v === '' || (Array.isArray(v) ? v.length === 0 : typeof v === 'object' ? Object.values(v as object).every(isBlank) : false);

/** The YAML a definition is written as (for copying into a repository); a small writer for this schema's shapes. */
export function definitionYaml(d: EntityDefinition): string {
  const out: string[] = [];
  const scalar = (v: unknown) => {
    const s = String(v);
    return /^[A-Za-z0-9_./@#-][A-Za-z0-9_ ./@#:-]*$/.test(s) && !/:\s|\s$|^(true|false|null|yes|no|\d+(\.\d+)?)$/i.test(s) ? s : JSON.stringify(s);
  };
  const write = (v: unknown, indent: string) => {
    if (Array.isArray(v)) {
      for (const item of v) {
        if (item && typeof item === 'object') {
          const lines: string[] = [];
          const sub = out.length;
          write(item, `${indent}  `);
          lines.push(...out.splice(sub));
          out.push(`${indent}- ${lines[0].trimStart()}`, ...lines.slice(1));
        } else out.push(`${indent}- ${scalar(item)}`);
      }
      return;
    }
    for (const [k, val] of Object.entries(v as Record<string, unknown>)) {
      if (isBlank(val)) continue;
      if (typeof val === 'object') {
        out.push(`${indent}${k}:`);
        write(val, `${indent}  `);
      } else out.push(`${indent}${k}: ${scalar(val)}`);
    }
  };
  write(d, '');
  return `${out.join('\n')}\n`;
}
