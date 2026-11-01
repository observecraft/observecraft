// Finds a logo for each integration of the integrations-core catalog in the two CC0 sets the app
// already uses: SVG Logos (@iconify-json/logos, full colour, preferred) and Simple Icons (one
// colour, drawn in the brand's colour). integrations-core has no logos of its own (Datadog's
// tiles load them from its CDN, which we don't use). Used by build-integration-logos.mjs.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

/** Brands whose names don't match their logo's: integration id → `logos:<name>` or `simple-icons:<slug>`. */
export const ALIASES = {
  postgres: 'logos:postgresql',
  pgbouncer: 'logos:postgresql',
  'kafka-consumer': 'logos:kafka-icon',
  'amazon-kafka': 'logos:kafka-icon',
  'confluent-platform': 'logos:kafka-icon',
  strimzi: 'logos:kafka-icon',
  cockroachdb: 'simple-icons:cockroachlabs',
  scylla: 'simple-icons:scylladb',
  hive: 'simple-icons:apachehive',
  'eks-anywhere': 'simple-icons:amazoneks',
  'eks-fargate': 'simple-icons:amazoneks',
  glusterfs: 'logos:redhat-icon',
  'hyper-v': 'logos:microsoft-icon',
  'cloud-foundry-api': 'simple-icons:cloudfoundry',
  vsphere: 'logos:vmware',
  esxi: 'logos:vmware',
  'hugging-face-tgi': 'logos:hugging-face-icon',
  druid: 'simple-icons:apachedruid',
  'hdfs-datanode': 'logos:hadoop',
  'hdfs-namenode': 'logos:hadoop',
  mapreduce: 'logos:hadoop',
  'jboss-wildfly': 'logos:wildfly',
  'snmp-f5': 'simple-icons:f5',
  'snmp-juniper': 'simple-icons:junipernetworks',
  'juniper-srx-firewall': 'simple-icons:junipernetworks',
  'snmp-cisco': 'simple-icons:cisco',
  'pan-firewall': 'simple-icons:paloaltonetworks',
  'palo-alto-cortex-xdr': 'simple-icons:paloaltonetworks',
  'palo-alto-networks-cortex-xsoar': 'simple-icons:paloaltonetworks',
  'palo-alto-panorama': 'simple-icons:paloaltonetworks',
  'idira-endpoint-privilege-manager': 'simple-icons:paloaltonetworks',
  twistlock: 'simple-icons:paloaltonetworks',
  fluxcd: 'simple-icons:flux',
  'consul-connect': 'logos:consul',
  'cassandra-nodetool': 'logos:cassandra',
  'nginx-ingress-controller': 'logos:nginx',
  torchserve: 'logos:pytorch-icon',
  'incident-io': 'logos:incident-icon',
  // Kubernetes' own components.
  kubelet: 'logos:kubernetes',
  'kube-dns': 'logos:kubernetes',
  'kube-proxy': 'logos:kubernetes',
  // Microsoft products: the vendor's logo.
  iis: 'logos:microsoft-icon',
  'active-directory': 'logos:microsoft-icon',
  'exchange-server': 'logos:microsoft-icon',
  'sql-server': 'logos:microsoft-icon',
  aspdotnet: 'logos:dotnet',
  'windows-service': 'logos:microsoft-windows-icon',
};

/** Integrations that must not take a logo found by name (a word they share with another brand). */
export const DENY = new Set(['container', 'directory', 'process', 'disk', 'battery', 'network', 'system-core', 'system-swap', 'obsidian-security']);

/** First words of product names that are not a brand on their own (Cloud Foundry is not Google Cloud). */
const COMMON_WORDS = new Set([
  'cloud',
  'network',
  'networks',
  'system',
  'kube',
  'data',
  'open',
  'active',
  'container',
  'storage',
  'map',
  'red',
  'power',
  'push',
  'zero',
  'traffic',
  'supply',
  'mac',
  'go',
  'do',
  'tcp',
  'http',
  'dns',
  'agent',
  'apache',
]);

/** Vendor prefixes a logo's name may carry that the integration's doesn't (Flink → apache-flink). */
const PREFIXES = ['', 'apache', 'microsoft', 'amazon', 'aws', 'azure', 'google', 'ibm', 'oracle', 'vmware', 'hashicorp'];

const norm = (s) =>
  s
    .toLowerCase()
    .replace(/\+/g, 'plus')
    .replace(/\./g, 'dot')
    .replace(/[^a-z0-9]/g, '');

export function loadLibraries() {
  const logos = JSON.parse(readFileSync(require.resolve('@iconify-json/logos/icons.json'), 'utf8'));
  const simpleIcons = Object.values(require('simple-icons')).filter((s) => s && s.slug);
  // The square "-icon" variant of a logo fits a tile better than its wordmark.
  const logosByKey = new Map();
  for (const name of Object.keys(logos.icons)) {
    const key = norm(name.replace(/-(icon|wordmark)$/, ''));
    if (!logosByKey.has(key) || name.endsWith('-icon')) logosByKey.set(key, name);
  }
  const simpleBySlug = new Map(simpleIcons.map((s) => [s.slug, s]));
  const simpleByKey = new Map();
  for (const s of simpleIcons) {
    simpleByKey.set(norm(s.title), s);
    simpleByKey.set(s.slug, s);
  }
  return { logos, logosByKey, simpleBySlug, simpleByKey };
}

/** The logo of one catalog integration ({ id, name }), or null. */
export function matchIntegrationLogo(integration, libraries) {
  const alias = ALIASES[integration.id];
  if (alias) {
    const [source, name] = alias.split(':');
    const found = source === 'logos' ? libraries.logos.icons[name] : libraries.simpleBySlug.get(name);
    if (!found) throw new Error(`Alias of ${integration.id}: ${alias} is in neither set`);
    return { source, name };
  }
  if (DENY.has(integration.id)) return null;
  const name = integration.name.replace(/\(.*?\)/g, '').trim();
  const whole = [integration.id, integration.id.replace(/^snmp-/, ''), integration.name, name].map(norm).filter((k) => k.length >= 3);
  const found = lookup(
    whole.flatMap((base) => PREFIXES.map((prefix) => prefix + base)),
    libraries,
  );
  if (found) return found;
  // A product of a vendor ("Cisco ASA", "Okta Workflows"): the vendor's logo, when its first word is a brand.
  const vendor = norm(name.split(/[\s/-]+/)[0] ?? '');
  return vendor.length >= 3 && !COMMON_WORDS.has(vendor)
    ? lookup(
        PREFIXES.map((prefix) => prefix + vendor),
        libraries,
      )
    : null;
}

/** Datadog's logo is never ours to show (CLAUDE.md): its components get a plain icon instead. */
const isDatadog = (key) => key.startsWith('datadog');

function lookup(keys, libraries) {
  keys = keys.filter((k) => !isDatadog(k));
  const logo = keys.find((k) => libraries.logosByKey.has(k));
  if (logo) return { source: 'logos', name: libraries.logosByKey.get(logo) };
  const simple = keys.find((k) => libraries.simpleByKey.has(k));
  if (simple) return { source: 'simple-icons', name: libraries.simpleByKey.get(simple).slug };
  return null;
}
