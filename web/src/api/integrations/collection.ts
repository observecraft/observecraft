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

/** Whether ObserveCraft receives this integration's data at all. */
export function availableHere(c: CollectionInfo): boolean {
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

/** The intake URL a vendor's webhook posts its events to, as logs with the integration's source. */
export function webhookUrl(intake: string, logSource: string): string {
  return `${intake.replace(/\/+$/, '')}/api/v2/logs?dd-api-key=<INGEST_API_KEY>&ddsource=${encodeURIComponent(logSource)}`;
}

/** ObserveCraft's own integration pages, for the tiles they cover. */
const OWN: Record<string, { to: string; label: string }> = {
  aws: { to: '/integrations/aws', label: 'Amazon Web Services' },
  azure: { to: '/integrations/azure', label: 'Microsoft Azure' },
  'google-cloud': { to: '/integrations/gcp', label: 'Google Cloud' },
  authentik: { to: '/integrations/authentik', label: 'authentik' },
  netbird: { to: '/integrations/netbird', label: 'NetBird' },
  fleetdm: { to: '/integrations/fleetdm', label: 'FleetDM' },
  slack: { to: '/integrations/slack', label: 'Slack' },
  // Datadog's documentation tiles for Kubernetes distributions: the Agent's Helm chart, which the Kubernetes page sets up.
  ...Object.fromEntries(
    ['amazon-eks', 'amazon-eks-blueprints', 'eks-anywhere', 'gke', 'oke', 'pivotal-pks', 'datadog-operator'].map((id) => [id, { to: '/integrations/kubernetes', label: 'Kubernetes' }]),
  ),
};

export function ownIntegration(id: string): { to: string; label: string } | undefined {
  return OWN[id];
}
