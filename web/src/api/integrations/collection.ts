// How Datadog collects each catalog tile (scripts/import-integration-catalog.mjs `collectionOf`), and what that means
// here: Agent checks, Agent logs and webhooks work as they are; senders with a free-form URL work once pointed at the
// intake; what Datadog's own service collects (vendor APIs, OAuth connections) or only documents does not.

/** See COLLECTIONS in scripts/import-integration-catalog.mjs. */
export type Collection = 'agent' | 'logs' | 'crawler' | 'oauth' | 'webhook' | 'push-site' | 'docs';

export interface CollectionInfo {
  /** Absent for ObserveCraft's own entries (OpenTelemetry templates, clouds, notification channels). */
  collection?: Collection;
  /** A push-site sender that also takes any URL, so it can send to the ObserveCraft intake. */
  customEndpoint?: boolean;
}

/** Collected by Datadog's own service, or not at all: the server refuses to install these (integration_installs.rs). */
const NOT_INSTALLABLE: Collection[] = ['crawler', 'oauth', 'docs'];

export function installable(collection: Collection | undefined): boolean {
  return !collection || !NOT_INSTALLABLE.includes(collection);
}

/** A connector making a tile available (docs/connectors.md): its id and page. */
export interface TileConnector {
  id: string;
  name: string;
  page: string;
  kind: 'package' | 'native' | 'push';
}

/** Connectors by the tiles they make available: the bundled packages (registered by mock/connectors.ts) and, live,
 * every connector the server lists (third-party ones included, integrations/client.ts). */
const CONNECTOR_TILES = new Map<string, TileConnector>();

export function registerConnector(c: { id: string; name: string; page?: string | null; kind: TileConnector['kind']; tiles: string[] }) {
  const entry = { id: c.id, name: c.name, page: c.page ?? `/integrations/connectors/${c.id}`, kind: c.kind };
  for (const tile of [c.id, ...c.tiles]) CONNECTOR_TILES.set(tile, entry);
}

/** The connector making a tile available, if any. */
export function tileConnector(id: string): TileConnector | undefined {
  return CONNECTOR_TILES.get(id);
}

/** Whether ObserveCraft receives this integration's data at all (`id`: a tile a connector covers is available). */
export function availableHere(c: CollectionInfo, id?: string): boolean {
  if (id && CONNECTOR_TILES.has(id)) return true;
  if (c.collection === 'push-site') return !!c.customEndpoint;
  return installable(c.collection);
}

/** One line on why an integration isn't available, for its tile. */
export function unavailableReason(name: string, c: CollectionInfo): string {
  switch (c.collection) {
    case 'crawler':
      return `Datadog collects this by calling ${name}'s API from its own service.`;
    case 'oauth':
      return `Datadog collects this through an OAuth connection between ${name} and its own service.`;
    case 'docs':
      return "Datadog's tile is documentation only: there is nothing to install for it.";
    case 'push-site':
      return `${name} sends its data only to Datadog's own sites, so it can't be pointed at ObserveCraft.`;
    default:
      return '';
  }
}

/** Vendors whose alert webhooks Datadog turns into events (`/intake/webhook/<name>`), not logs. */
const EVENT_WEBHOOKS = new Set(['solarwinds']);

/** The intake URL a vendor's webhook posts to: as logs with the integration's source (the query applies it to every
 * log), or as events for the alert webhooks Datadog makes events of. */
export function webhookUrl(intake: string, logSource: string): string {
  const base = intake.replace(/\/+$/, '');
  if (EVENT_WEBHOOKS.has(logSource)) return `${base}/intake/webhook/${logSource}?api_key=<INGEST_API_KEY>`;
  return `${base}/api/v2/logs?dd-api-key=<INGEST_API_KEY>&ddsource=${encodeURIComponent(logSource)}`;
}

/** ObserveCraft's own integration pages, for the tiles they cover. */
const OWN: Record<string, { to: string; label: string }> = {
  aws: { to: '/integrations/aws', label: 'Amazon Web Services' },
  azure: { to: '/integrations/azure', label: 'Microsoft Azure' },
  // Entra ID's sign-in, audit and risk logs: read from Microsoft Graph with an Azure app registration.
  'azure-active-directory': { to: '/integrations/azure?detail=entra', label: 'Entra ID logs' },
  'google-cloud': { to: '/integrations/gcp', label: 'Google Cloud' },
  authentik: { to: '/integrations/authentik', label: 'authentik' },
  fleetdm: { to: '/integrations/fleetdm', label: 'FleetDM' },
  'gitlab-audit-events': { to: '/integrations/gitlab-audit-events', label: 'GitLab Audit Events' },
  'google-workspace': { to: '/integrations/google-workspace', label: 'Google Workspace' },
  slack: { to: '/integrations/slack', label: 'Slack' },
  // Datadog's documentation tiles for Kubernetes distributions: the Agent's Helm chart, which the Kubernetes page sets up.
  ...Object.fromEntries(
    ['amazon-eks', 'amazon-eks-blueprints', 'eks-anywhere', 'gke', 'oke', 'pivotal-pks', 'datadog-operator'].map((id) => [id, { to: '/integrations/kubernetes', label: 'Kubernetes' }]),
  ),
};

export function ownIntegration(id: string): { to: string; label: string } | undefined {
  const c = CONNECTOR_TILES.get(id);
  return OWN[id] ?? (c ? { to: c.page, label: c.name } : undefined);
}
