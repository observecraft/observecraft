// Resolves a real logo for every integration of the catalog: the checks imported from integrations-core
// (src/api/integrations/catalog.generated.json) and the services of the AWS, Azure and Google Cloud
// integrations (server/seeds/*_integration.json). Sources are SVG Logos (full colour, preferred) and
// Simple Icons (one colour), both CC0. Used by build-logos.mjs; tested by scripts/integration-logos.test.mjs.
import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as simpleIcons from 'simple-icons';
import { BRANDS } from './fetch-brand-icons.mjs';

const require = createRequire(import.meta.url);
const json = (path) => JSON.parse(readFileSync(new URL(path, import.meta.url), 'utf8'));

export const SVG_LOGOS = JSON.parse(readFileSync(require.resolve('@iconify-json/logos/icons.json'), 'utf8'));
export const SIMPLE_ICONS = new Map(
  Object.values(simpleIcons)
    .filter((i) => i?.slug)
    .map((i) => [i.slug, i]),
);
const hasSvgLogo = (name) => name in SVG_LOGOS.icons || name in (SVG_LOGOS.aliases ?? {});
/** Downloaded logos (fetch-brand-icons.mjs): `brand:<name>` → its SVG or PNG file, if present. */
export function brandIconFile(ref) {
  const base = `../src/assets/brand-icons/${ref.slice('brand:'.length)}`;
  for (const ext of ['svg', 'png']) {
    const file = new URL(`${base}.${ext}`, import.meta.url);
    if (existsSync(file)) return file;
  }
  return null;
}
const BRAND_OF = new Map(Object.entries(BRANDS).flatMap(([name, b]) => b.ids.map((id) => [id, `brand:${name}`])));

/** The providers' official service icons (fetch-cloud-icons.mjs): `cloud:<cloud>/<id>` → its SVG file. */
export const cloudIconFile = (ref) => new URL(`../src/assets/cloud-icons/${ref.slice('cloud:'.length)}.svg`, import.meta.url);

/** Integrations whose id or name doesn't lead to their logo: id → `logos:<SVG Logos name>` or `si:<Simple Icons slug>`. */
const OVERRIDES = {
  // Simple Icons' "hive" is a blockchain; Argo CD has no id of its own there
  hive: 'si:apachehive',
  argocd: 'logos:argo-icon',
  postgres: 'logos:postgresql',
  'active-directory': 'logos:microsoft-icon',
  'avi-vantage': 'logos:vmware',
  marathon: 'logos:mesos',
  'pan-firewall': 'si:paloaltonetworks',
  scylla: 'si:scylladb',
  fluxcd: 'logos:flux',
  boundary: 'logos:hashicorp-icon',
  torchserve: 'logos:pytorch-icon',
  // Apache projects without a mark of their own in either set: the ASF feather
  activemq: 'logos:apache',
  'activemq-xml': 'logos:apache',
  zookeeper: 'logos:apache',
  ignite: 'logos:apache',
  hudi: 'logos:apache',
  'traffic-server': 'logos:apache',
  // Hadoop family (Simple Icons' "yarn" is the JavaScript package manager)
  yarn: 'si:apachehadoop',
  mapreduce: 'si:apachehadoop',
  'hdfs-datanode': 'si:apachehadoop',
  'hdfs-namenode': 'si:apachehadoop',
  mapr: 'si:apachehadoop',
  cloudera: 'si:cloudera',
  ambari: 'si:apachehadoop',
  cockroachdb: 'si:cockroachlabs',
  openshift: 'si:redhatopenshift',
  glusterfs: 'logos:redhat-icon',
  vsphere: 'logos:vmware',
  esxi: 'logos:vmware',
  'cassandra-nodetool': 'logos:cassandra',
  'kafka-consumer': 'logos:kafka-icon',
  'amazon-kafka': 'logos:aws-msk',
  'confluent-platform': 'logos:kafka-icon',
  strimzi: 'logos:kafka-icon',
  streamnative: 'si:apachepulsar',
  'aws-fargate': 'logos:aws-fargate',
  'eks-fargate': 'logos:aws-fargate',
  'amazon-eks': 'logos:aws-eks',
  'amazon-eks-blueprints': 'logos:aws-eks',
  'eks-anywhere': 'logos:aws-eks',
  'amazon-app-mesh': 'logos:aws-app-mesh',
  'iam-access-analyzer': 'logos:aws-iam',
  'aws-neuron': 'logos:aws',
  // Simple Icons dropped Amazon's logo in v16
  'amazon-vpc-cni': 'logos:aws',
  gke: 'logos:kubernetes',
  oke: 'logos:kubernetes',
  'pivotal-pks': 'logos:kubernetes',
  'azure-iot-edge': 'logos:microsoft-azure',
  'azure-active-directory': 'logos:microsoft-azure',
  'microsoft-dns': 'logos:microsoft-windows-icon',
  'microsoft-sysmon': 'logos:microsoft-windows-icon',
  'microsoft-copilot': 'logos:microsoft-icon',
  'exchange-server': 'logos:microsoft-icon',
  'sql-server': 'logos:microsoft-icon',
  iis: 'logos:microsoft-windows-icon',
  pdh: 'logos:microsoft-windows-icon',
  wmi: 'logos:microsoft-windows-icon',
  'event-viewer': 'logos:microsoft-windows-icon',
  wincrashdetect: 'logos:microsoft-windows-icon',
  winkmem: 'logos:microsoft-windows-icon',
  'hyper-v': 'logos:microsoft-windows-icon',
  aspdotnet: 'logos:dotnet',
  dotnetclr: 'logos:dotnet',
  'linux-audit-logs': 'logos:linux-tux',
  'linux-proc-extras': 'logos:linux-tux',
  btrfs: 'logos:linux-tux',
  journald: 'logos:linux-tux',
  systemd: 'logos:linux-tux',
  'mac-audit-logs': 'logos:apple',
  'nvidia-jetson': 'logos:nvidia',
  'nvidia-nim': 'logos:nvidia',
  'nvidia-triton': 'logos:nvidia',
  'kai-scheduler': 'logos:nvidia',
  dcgm: 'logos:nvidia',
  'anthropic-usage-and-costs': 'logos:anthropic-icon',
  'anthropic-compliance-logs': 'logos:anthropic-icon',
  'gitlab-runner': 'logos:gitlab',
  'php-fpm': 'logos:php',
  'go-expvar': 'logos:go',
  'go-metro': 'logos:go',
  'jboss-wildfly': 'logos:wildfly',
  weblogic: 'logos:oracle',
  'nginx-ingress-controller': 'logos:nginx',
  'consul-connect': 'logos:consul',
  // Shown as Traefik (the tile covers Traefik Proxy too): Traefik Proxy's own mark
  'traefik-mesh': 'si:traefikproxy',
  // integrations-extras
  filebeat: 'logos:beats',
  nvml: 'logos:nvidia',
  flume: 'logos:apache',
  redisenterprise: 'logos:redis',
  'unifi-console': 'si:ubiquiti',
  cfssl: 'logos:cloudflare-icon',
  'loadrunner-professional': 'si:opentext',
  lacework: 'si:fortinet',
  'robust-intelligence-ai-firewall': 'si:cisco',
  ns1: 'logos:ibm',
  'kubernetes-state': 'logos:kubernetes',
  'kubernetes-state-core': 'logos:kubernetes',
  kubelet: 'logos:kubernetes',
  'external-dns': 'logos:kubernetes',
  karpenter: 'logos:kubernetes',
  'cert-manager': 'logos:kubernetes',
  'kube-dns': 'logos:kubernetes',
  'cri-o': 'logos:kubernetes',
  cri: 'logos:kubernetes',
  'snmp-dell': 'si:dell',
  'snmp-hewlett-packard-enterprise': 'si:hp',
  'snmp-juniper': 'si:junipernetworks',
  'juniper-srx-firewall': 'si:junipernetworks',
  'snmp-american-power-conversion': 'si:schneiderelectric',
  twistlock: 'si:paloaltonetworks',
  'palo-alto-panorama': 'si:paloaltonetworks',
  'palo-alto-cortex-xdr': 'si:paloaltonetworks',
  'palo-alto-networks-cortex-xsoar': 'si:paloaltonetworks',
  'idira-endpoint-privilege-manager': 'si:paloaltonetworks',
  'trend-micro-email-security': 'si:trendmicro',
  'trend-micro-vision-one-endpoint-security': 'si:trendmicro',
  'trend-micro-vision-one-xdr': 'si:trendmicro',
  'sap-hana': 'logos:sap',
  'fly-io': 'si:flydotio',
  'incident-io': 'logos:incident-icon',
  'hubspot-content-hub': 'logos:hubspot',
  'do-query-actions': 'logos:digital-ocean',
  'temporal-cloud': 'si:temporal',
  'supabase-cloud': 'logos:supabase-icon',
  'cloud-foundry-api': 'si:cloudfoundry',
  'openstack-controller': 'logos:openstack-icon',
  'okta-workflows': 'logos:okta-icon',
  'have-i-been-pwned': 'si:haveibeenpwned',
  'riak-cs': 'logos:riak',
  'kubevirt-api': 'logos:kubernetes',
  'kubevirt-controller': 'logos:kubernetes',
  'kubevirt-handler': 'logos:kubernetes',
  'citrix-hypervisor': 'si:citrix',
  'aws-applicationelb': 'logos:aws-elb',
  'aws-networkelb': 'logos:aws-elb',
  'aws-elb': 'logos:aws-elb',
  'aws-kafka': 'logos:aws-msk',
  'aws-states': 'logos:aws-step-functions',
  'aws-x-ray': 'logos:aws-xray',
  'aws-logs': 'logos:aws-cloudwatch',
  'aws-networkmonitor': 'logos:aws-cloudwatch',
  'aws-docdb': 'logos:aws-documentdb',
  'aws-elasticbeanstalk': 'logos:aws-elastic-beanstalk',
  'aws-certificatemanager': 'logos:aws-certificate-manager',
  'aws-events': 'logos:aws-eventbridge',
  'aws-eventbridge-pipes': 'logos:aws-eventbridge',
  'aws-scheduler': 'logos:aws-eventbridge',
  'aws-cassandra': 'logos:aws-keyspaces',
  'aws-amazonmq': 'logos:aws-mq',
  'aws-ddosprotection': 'logos:aws-shield',
  'aws-wafv2': 'logos:aws-waf',
  'aws-route53resolver': 'logos:aws-route53',
  'aws-rds-proxy': 'logos:aws-rds',
  'aws-dax': 'logos:aws-dynamodb',
  'aws-ec2-api': 'logos:aws-ec2',
  'aws-ec2-infrastructureperformance': 'logos:aws-ec2',
  'aws-ec2spot': 'logos:aws-ec2',
  'aws-autoscaling': 'logos:aws-ec2',
  'aws-ebs': 'logos:aws-ec2',
  'aws-es': 'logos:aws-open-search',
  'aws-aoss': 'logos:aws-open-search',
  'aws-firehose': 'logos:aws-kinesis',
  'aws-kinesisanalytics': 'logos:aws-kinesis',
  'aws-natgateway': 'logos:aws-vpc',
  'aws-vpn': 'logos:aws-vpc',
  'aws-transitgateway': 'logos:aws-vpc',
  'aws-privatelinkendpoints': 'logos:aws-vpc',
  'aws-privatelinkservices': 'logos:aws-vpc',
  'aws-ses': 'logos:aws-ses',
  'aws-iot': 'logos:aws',
  'gcp-gke': 'logos:kubernetes',
  'gcp-gcs': 'si:googlecloudstorage',
  'gcp-bigquery': 'si:googlebigquery',
  'gcp-bigtable': 'si:googlebigtable',
  'gcp-spanner': 'si:googlecloudspanner',
  'gcp-pubsub': 'si:googlepubsub',
  'gcp-dataflow': 'si:googledataflow',
  'gcp-dataproc': 'si:googledataproc',
  'gcp-composer': 'si:googlecloudcomposer',
  'gcp-run': 'logos:google-cloud-run',
  'gcp-cloudfunctions': 'logos:google-cloud-functions',
  'gcp-aiplatform': 'logos:google-gemini',
  'gcp-managedkafka': 'logos:kafka-icon',
  'azure-containerservice-managedclusters': 'logos:kubernetes',
  'azure-databricks-workspaces': 'si:databricks',
  'azure-cache-redis': 'logos:redis',
  'azure-dbforpostgresql-flexibleservers': 'logos:postgresql',
  'azure-dbformysql-flexibleservers': 'logos:mysql-icon',
  'azure-powerbidedicated-capacities': 'logos:microsoft-power-bi',
  // Datadog's own open-source products (Apache-2.0) carry Datadog's logo: the product is theirs (CLAUDE.md, licenses).
  'datadog-agent': 'logos:datadog-icon',
  'datadog-cluster-agent': 'logos:datadog-icon',
  'datadog-operator': 'logos:datadog-icon',
  guarddog: 'logos:datadog-icon',
  'ide-shepherd': 'logos:datadog-icon',
  'supply-chain-firewall': 'logos:datadog-icon',
};

/** Checks of the host itself: shown with our tile, never a third-party brand. */
const GENERIC = new Set([
  'disk',
  'network',
  'process',
  'battery',
  'directory',
  'tls',
  'ntp',
  'dns',
  'ssh',
  'thermal',
  'container',
  'system-core',
  'system-swap',
  'tcp-check',
  'http-check',
  'tcp-queue-length',
  'oom-kill',
  'nfsstat',
  'infiniband',
  'wlan',
  'storage-management',
  'snmp',
  'statsd',
  'openmetrics',
  'lparstats',
  // integrations-extras checks of protocols and host states, not of a product
  'ping',
  'reboot-required',
  'snmpwalk',
  'upsc',
  'sortdb',
  'hikaricp',
  'sendmail',
]);
/** Whether an integration is a host check or Datadog's own agent, shown with our tile on purpose. */
export const isGeneric = (id) => GENERIC.has(id);
/** Simple Icons' brands we never show for a match on a word: our own competitor's mark and common words. */
const NEVER = new Set(['datadog', 'process', 'network', 'container', 'directory', 'battery', 'thermal']);
/** Provider prefixes of the cloud services: their services fall back to the provider's logo. */
const PROVIDER = { aws: 'logos:aws', azure: 'logos:microsoft-azure', gcp: 'logos:google-cloud', kube: 'logos:kubernetes', windows: 'logos:microsoft-windows-icon', snmp: null };

const slug = (text) =>
  text
    .toLowerCase()
    .replace(/\+/g, 'plus')
    .replace(/\./g, 'dot')
    .replace(/&/g, 'and')
    .replace(/[^a-z0-9]/g, '');

/** The logo of one name: SVG Logos (`x-icon` first, square marks suit tiles) then Simple Icons. */
function byName(name) {
  for (const n of [`${name}-icon`, name]) if (hasSvgLogo(n)) return `logos:${n}`;
  const s = slug(name);
  if (!NEVER.has(s) && SIMPLE_ICONS.has(s)) return `si:${s}`;
  if (!NEVER.has(s) && SIMPLE_ICONS.has(`apache${s}`)) return `si:apache${s}`;
  return null;
}

/** Whether a `logos:<name>` / `si:<slug>` reference names an icon of its set. */
export function logoExists(ref) {
  const [set, name] = ref.split(':');
  if (set === 'cloud') return existsSync(cloudIconFile(ref));
  if (set === 'brand') return brandIconFile(ref) !== null;
  return set === 'logos' ? hasSvgLogo(name) : set === 'si' && SIMPLE_ICONS.has(name);
}

/** `cloud:<cloud>/<id>`, `brand:<name>`, `logos:<name>`, `si:<slug>` or null (our own tile) for one catalog integration `{ id, name }`. */
export function resolveLogo({ id, name, cloud }) {
  if (cloud && logoExists(`cloud:${cloud}/${id}`)) return `cloud:${cloud}/${id}`;
  if (id in OVERRIDES) return OVERRIDES[id];
  if (GENERIC.has(id)) return null;
  const brand = BRAND_OF.get(id);
  if (brand && logoExists(brand)) return brand;
  const exact = byName(id) ?? byName(name);
  if (exact) return exact;
  // Vendor products (`cisco-secure-endpoint`, `snmp-fortinet`): the longest prefix with a logo, then the provider.
  const parts = id.split('-');
  if (parts[0] === 'snmp') parts.shift();
  for (let n = parts.length - 1; n >= 1; n--) {
    const prefix = parts.slice(0, n).join('-');
    if (prefix in PROVIDER) return PROVIDER[prefix];
    const found = byName(prefix);
    if (found) return found;
  }
  if (parts.length === 1 && parts[0] !== id) return byName(parts[0]);
  return null;
}

/** Every catalog integration with the logo key the UI asks for (see mock/integrations.ts). */
export function catalogLogoKeys() {
  const core = json('../src/api/integrations/catalog.generated.json').integrations.map((c) => ({ key: c.id, id: c.id, name: c.name }));
  const aws = json('../../server/seeds/aws_integration.json').namespaces.map((n) => ({ key: `tile:${n.id}`, id: n.id, name: n.name, cloud: 'aws' }));
  const azure = json('../../server/seeds/azure_integration.json').resource_types.map((t) => ({ key: `tile:${t.id}`, id: t.id, name: t.name, cloud: 'azure' }));
  const gcp = json('../../server/seeds/gcp_integration.json').services.map((s) => ({ key: `tile:${s.id}`, id: s.id, name: s.name, cloud: 'gcp' }));
  return [...core, ...aws, ...azure, ...gcp];
}
