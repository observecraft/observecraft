// Downloads the official AWS, Azure and Google Cloud architecture icon packages and copies the icon of
// every service of our cloud integrations (server/seeds/*_integration.json) to src/assets/cloud-icons/.
// The icons are the providers' property, used with the owner's approval to identify the services our
// integrations monitor; their terms are in src/assets/cloud-icons/NOTICE.md. build-logos.mjs embeds them.
// Run: node scripts/fetch-cloud-icons.mjs   (then npm run logos)
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';

export const PACKAGES = {
  aws: ['https://d1.awsstatic.com/onedam/marketing-channels/website/public/shared/architecture-icon-release/Icon-package_07312026.5846e92413caa21490223536cc97f1269e44fa92.zip'],
  azure: ['https://arch-center.azureedge.net/icons/Azure_Public_Service_Icons_V24.zip'],
  gcp: ['https://services.google.com/fh/files/misc/core-products-icons.zip', 'https://services.google.com/fh/files/misc/google-cloud-legacy-icons.zip'],
};

/** Lowercase letters and digits only, without the provider's prefix words. */
export const normalize = (text) =>
  text
    .toLowerCase()
    .replace(/\b(amazon|aws|azure|microsoft|google|cloud|gcp)\b/g, '')
    .replace(/[^a-z0-9]/g, '');

/** Our service id → the package icon's normalized name, where the names differ. */
const ALIASES = {
  'aws-applicationelb': 'elasticloadbalancing',
  'aws-elb': 'elasticloadbalancing',
  'aws-networkelb': 'elasticloadbalancing',
  'aws-autoscaling': 'ec2autoscaling',
  'aws-billing': 'billingconductor',
  'aws-budgeting': 'budgets',
  'aws-usage': 'costexplorer',
  'aws-logs': 'cloudwatch',
  'aws-networkmonitor': 'cloudwatch',
  'aws-dx': 'directconnect',
  'aws-docdb': 'documentdb',
  'aws-dax': 'dynamodb',
  'aws-ec2-api': 'ec2',
  'aws-ec2-infrastructureperformance': 'ec2',
  'aws-ec2spot': 'ec2',
  'aws-elasticmapreduce': 'emr',
  'aws-ebs': 'elasticblockstore',
  'aws-ecr': 'elasticcontainerregistry',
  'aws-efs': 'efs',
  'aws-elasticinference': 'sagemakerai',
  'aws-events': 'eventbridge',
  'aws-eventbridge-pipes': 'eventbridge',
  'aws-scheduler': 'eventbridge',
  'aws-iot': 'iotcore',
  'aws-kms': 'keymanagementservice',
  'aws-cassandra': 'keyspaces',
  'aws-kinesis': 'kinesisdatastreams',
  'aws-kinesisanalytics': 'managedserviceforapacheflink',
  'aws-firehose': 'datafirehose',
  'aws-amazonmq': 'mq',
  'aws-ml': 'sagemakerai',
  'aws-kafka': 'managedstreamingforapachekafka',
  'aws-amazonmwaa': 'managedworkflowsforapacheairflow',
  'aws-natgateway': 'virtualprivate',
  'aws-ecs': 'elasticcontainerservice',
  'aws-ecs-containerinsights': 'elasticcontainerservice',
  'aws-gamelift': 'gameliftservers',
  'aws-opsworks': 'systemsmanager',
  'aws-servicequotas': 'systemsmanager',
  'aws-network-manager': 'wan',
  'aws-es': 'opensearchservice',
  'aws-aoss': 'opensearchservice',
  'aws-privatelinkendpoints': 'privatelink',
  'aws-privatelinkservices': 'privatelink',
  'aws-rds-proxy': 'rds',
  'aws-route53resolver': 'route53',
  'aws-s3-storage-lens': 'simplestorageservice',
  'aws-sagemaker': 'sagemakerai',
  'aws-sagemaker-endpoints': 'sagemakerai',
  'aws-sagemaker-labelingjobs': 'sagemakergroundtruth',
  'aws-sagemaker-modelbuildingpipeline': 'sagemakerai',
  'aws-sagemaker-processingjobs': 'sagemakerai',
  'aws-sagemaker-trainingjobs': 'sagemakerai',
  'aws-sagemaker-transformjobs': 'sagemakerai',
  'aws-sagemaker-workteam': 'sagemakergroundtruth',
  'aws-ddosprotection': 'shield',
  'aws-ses': 'simpleemailservice',
  'aws-sns': 'simplenotificationservice',
  'aws-sqs': 'simplequeueservice',
  'aws-swf': 'stepfunctions',
  'aws-states': 'stepfunctions',
  'aws-vpn': 'sitetositevpn',
  'aws-waf': 'waf',
  'aws-wafv2': 'waf',
  'aws-x-ray': 'xray',
  'aws-elasticache': 'elasticache',
  'aws-codewhisperer': 'qdeveloper',
  'aws-ml-api': 'sagemakerai',
  'aws-ec2': 'ec2',
  'aws-s3': 'simplestorageservice',
  'aws-apigateway': 'apigateway',
  'azure-compute-virtualmachines': 'virtualmachine',
  'azure-compute-virtualmachinescalesets': 'vmscalesets',
  'azure-compute-disks': 'disks',
  'azure-web-sites': 'appservices',
  'azure-web-serverfarms': 'appserviceplans',
  'azure-web-hostingenvironments': 'appserviceenvironments',
  'azure-app-containerapps': 'workercontainerapp',
  'azure-containerinstance-containergroups': 'containerinstances',
  'azure-containerservice-managedclusters': 'kubernetesservices',
  'azure-containerregistry-registries': 'containerregistries',
  'azure-storage-storageaccounts': 'storageaccounts',
  'azure-sql-servers-databases': 'sqldatabase',
  'azure-sql-servers-elasticpools': 'sqlelasticpools',
  'azure-sql-managedinstances': 'sqlmanagedinstance',
  'azure-dbforpostgresql-flexibleservers': 'databasepostgresqlserver',
  'azure-dbformysql-flexibleservers': 'databasemysqlserver',
  'azure-documentdb-databaseaccounts': 'cosmosdb',
  'azure-cache-redis': 'cacheredis',
  'azure-cache-redisenterprise': 'managedredis',
  'azure-network-loadbalancers': 'loadbalancers',
  'azure-network-applicationgateways': 'applicationgateways',
  'azure-network-frontdoors': 'frontdoorandcdnprofiles',
  'azure-network-azurefirewalls': 'firewalls',
  'azure-network-natgateways': 'nat',
  'azure-network-publicipaddresses': 'publicipaddresses',
  'azure-network-virtualnetworkgateways': 'virtualnetworkgateways',
  'azure-network-expressroutecircuits': 'expressroutecircuits',
  'azure-network-dnszones': 'dnszones',
  'azure-network-privatednszones': 'dnszones',
  'azure-network-trafficmanagerprofiles': 'trafficmanagerprofiles',
  'azure-cdn-profiles': 'frontdoorandcdnprofiles',
  'azure-eventhub-namespaces': 'eventhubs',
  'azure-servicebus-namespaces': 'servicebus',
  'azure-eventgrid-topics': 'eventgridtopics',
  'azure-eventgrid-systemtopics': 'systemtopic',
  'azure-relay-namespaces': 'relays',
  'azure-logic-workflows': 'logicapps',
  'azure-automation-automationaccounts': 'automationaccounts',
  'azure-datafactory-factories': 'datafactories',
  'azure-synapse-workspaces': 'synapseanalytics',
  'azure-databricks-workspaces': 'databricks',
  'azure-hdinsight-clusters': 'hdinsightclusters',
  'azure-streamanalytics-streamingjobs': 'streamanalyticsjobs',
  'azure-search-searchservices': 'search',
  'azure-cognitiveservices-accounts': 'openai',
  'azure-machinelearningservices-workspaces': 'machinelearning',
  'azure-keyvault-vaults': 'keyvaults',
  'azure-apimanagement-service': 'apimanagementservices',
  'azure-appconfiguration-configurationstores': 'appconfiguration',
  'azure-signalrservice-signalr': 'signalr',
  'azure-devices-iothubs': 'iothub',
  'azure-batch-batchaccounts': 'batchaccounts',
  'azure-recoveryservices-vaults': 'recoveryservicesvaults',
  'azure-dataprotection-backupvaults': 'backupvault',
  'azure-insights-components': 'applicationinsights',
  'azure-operationalinsights-workspaces': 'loganalyticsworkspaces',
  'azure-insights-datacollectionrules': 'datacollectionrules',
  'azure-netapp-netappaccounts-capacitypools-volumes': 'netappfiles',
  'azure-analysisservices-servers': 'analysisservices',
  'azure-powerbidedicated-capacities': 'powerbiembedded',
  'gcp-gce': 'computeengine',
  'gcp-gcs': 'storage',
  'gcp-cloudsql': 'sql',
  'gcp-run': 'run',
  'gcp-cloudfunctions': 'functions',
  'gcp-pubsub': 'pubsub',
  'gcp-loadbalancing': 'loadbalancing',
  'gcp-gke': 'gke',
  'gcp-bigquery': 'bigquery',
  'gcp-bigtable': 'bigtable',
  'gcp-spanner': 'spanner',
  'gcp-firestore': 'firestore',
  'gcp-datastore': 'datastore',
  'gcp-redis': 'memorystore',
  'gcp-memcache': 'memorystore',
  'gcp-alloydb': 'alloydb',
  'gcp-dataflow': 'dataflow',
  'gcp-dataproc': 'dataproc',
  'gcp-composer': 'composer',
  'gcp-gae': 'appengine',
  'gcp-logging': 'logging',
  'gcp-router': 'router',
  'gcp-vpn': 'vpn',
  'gcp-interconnect': 'interconnect',
  'gcp-dns': 'dns',
  'gcp-networking': 'virtualprivate',
  'gcp-firewallinsights': 'firewallrules',
  'gcp-cloudtasks': 'tasks',
  'gcp-iam': 'identityandaccessmanagement',
  'gcp-serviceruntime': 'apis',
  'gcp-aiplatform': 'vertexai',
  'gcp-filestore': 'filestore',
  'gcp-artifactregistry': 'artifactregistry',
  'gcp-cloudbuild': 'build',
  'gcp-apigateway': 'apigateway',
  'gcp-workflows': 'workflows',
  'gcp-managedkafka': 'pubsub',
  'gcp-tpu': 'tpu',
  'gcp-memorystore': 'memorystore',
  'gcp-monitoring': 'monitoring',
  'gcp-pubsublite': 'pubsub',
  'gcp-bigquerystorage': 'bigquery',
  'gcp-bigquerydatatransfer': 'datatransfer',
  'gcp-storagetransfer': 'transfer',
  'gcp-netapp': 'filestore',
  'gcp-parallelstore': 'filestore',
  'gcp-lustre': 'filestore',
  'gcp-backupdr': 'persistentdisk',
  'gcp-vpcaccess': 'virtualprivate',
  'gcp-networkservices': 'network',
  'gcp-edgecache': 'cdn',
  'gcp-recaptchaenterprise': 'webrisk',
  'gcp-gkebackup': 'gke',
};

/** Moves `<style>` class rules onto the elements as inline styles: on a page, an icon's `.st1` would restyle every other icon. */
export function inlineStyles(svg) {
  const rules = new Map();
  for (const [, css] of svg.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g))
    for (const [, selectors, body] of css.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
      const decl = body
        .split(';')
        .map((d) => d.trim().replace(/\s*:\s*/, ':'))
        .filter(Boolean);
      for (const sel of selectors.split(',').map((x) => x.trim())) if (/^\.[\w-]+$/.test(sel)) rules.set(sel.slice(1), [...(rules.get(sel.slice(1)) ?? []), ...decl]);
    }
  // Each removal repeats until stable: removing one tag can join the text around it into another (`<sty<style></style>le>`).
  let unstyled = svg;
  let last;
  do {
    last = unstyled;
    unstyled = unstyled.replace(/<style\b[^>]*>[\s\S]*?<\/style\s*>\s*/gi, '');
  } while (unstyled !== last);
  do {
    last = unstyled;
    unstyled = unstyled.replace(/<\/?style\b[^>]*>/gi, '');
  } while (unstyled !== last);
  return unstyled.replace(/<([a-zA-Z]+)([^>]*?)\sclass="([^"]*)"([^>]*)>/g, (_, tag, before, classes, after) => {
    const decl = classes.split(/\s+/).flatMap((c) => rules.get(c) ?? []);
    const own = /\sstyle="([^"]*)"/.exec(before + after);
    if (own)
      decl.push(
        ...own[1]
          .split(';')
          .map((d) => d.trim())
          .filter(Boolean),
      );
    const rest = (before + after).replace(/\sstyle="[^"]*"/, '');
    const [attrs, close] = rest.endsWith('/') ? [rest.slice(0, -1), '/'] : [rest, ''];
    return `<${tag}${decl.length ? ` style="${decl.join(';')}"` : ''}${attrs}${close}>`;
  });
}

/** Every SVG below a directory. */
function svgs(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return svgs(path);
    return name.endsWith('.svg') && !name.startsWith('._') && !path.includes('__MACOSX') ? [path] : [];
  });
}

/** The icons of one provider's packages by normalized name (the preferred variant of each). */
export function iconIndex(cloud, files) {
  const index = new Map();
  const add = (name, path) => index.has(name) || index.set(name, path);
  if (cloud === 'aws') {
    // Architecture-Service-Icons_*/Arch_<Category>/64/Arch_<Service>_64.svg
    for (const f of files.filter((f) => /Architecture-Service-Icons.*\/64\/Arch_.*_64\.svg$/.test(f)))
      add(
        normalize(
          basename(f)
            .replace(/^Arch_|_64\.svg$/g, '')
            .replace(/-/g, ' '),
        ),
        f,
      );
  } else if (cloud === 'azure') {
    // Icons/<category>/<number>-icon-service-<Service>.svg
    for (const f of files.filter((f) => /-icon-service-/.test(f)))
      add(
        normalize(
          basename(f, '.svg')
            .replace(/^\d+-icon-service-/, '')
            .replace(/-/g, ' '),
        ),
        f,
      );
  } else {
    // Unique Icons/<Product>/SVG/<Product>-512-color*.svg first, then the legacy <product>/<product>.svg
    for (const f of files.filter((f) => /Unique Icons\/[^/]+\/SVG\/[^/]+-512-color(-rgb)?\.svg$/.test(f))) add(normalize(f.split('/').at(-3)), f);
    for (const f of files.filter((f) => !/Unique Icons/.test(f))) add(normalize(basename(f, '.svg').replace(/_/g, ' ')), f);
  }
  return index;
}

/** Icons the app shows beyond the integrations' services (the CI and source providers on Software Delivery and the portal). */
const EXTRAS = { aws: [], azure: [{ id: 'azure-devops', name: 'Azure DevOps' }], gcp: [] };

/** Our services of one cloud: `{ id, name }`. */
export function services(cloud) {
  const seed = (name) => JSON.parse(readFileSync(new URL(`../../server/seeds/${name}_integration.json`, import.meta.url), 'utf8'));
  if (cloud === 'aws') return [...seed('aws').namespaces, ...EXTRAS.aws];
  if (cloud === 'azure') return [...seed('azure').resource_types, ...EXTRAS.azure];
  return [...seed('gcp').services, ...EXTRAS.gcp];
}

/** The package icon of one service, by alias or by name. */
export function iconFor(index, { id, name }) {
  return index.get(ALIASES[id] ?? '') ?? index.get(normalize(name)) ?? null;
}

export const OUT = new URL('../src/assets/cloud-icons/', import.meta.url);

if (process.argv[1] === new URL(import.meta.url).pathname) {
  const work = process.argv[2] ?? mkdtempSync(join(tmpdir(), 'cloud-icons-'));
  const missing = [];
  for (const [cloud, urls] of Object.entries(PACKAGES)) {
    const dir = join(work, cloud);
    if (!existsSync(dir)) {
      mkdirSync(dir, { recursive: true });
      for (const [i, url] of urls.entries()) {
        const zip = join(work, `${cloud}-${i}.zip`);
        execFileSync('curl', ['-sSLf', '-o', zip, url]);
        execFileSync('unzip', ['-qo', zip, '-d', dir]);
      }
    }
    const index = iconIndex(cloud, svgs(dir));
    const out = new URL(`${cloud}/`, OUT);
    rmSync(out, { recursive: true, force: true });
    mkdirSync(out, { recursive: true });
    for (const s of services(cloud)) {
      const file = iconFor(index, s);
      if (!file) {
        missing.push(`${s.id} (${s.name})`);
        continue;
      }
      let svg = readFileSync(file, 'utf8')
        .replace(/<\?xml[^>]*>\s*/, '')
        .replace(/<title>[^<]*<\/title>/g, '');
      // Repeated until stable: removing one comment can join the text around it into another.
      let previous;
      do {
        previous = svg;
        svg = svg.replace(/<!--[\s\S]*?-->\s*/g, '');
      } while (svg !== previous);
      svg = svg.trim();
      svg = inlineStyles(svg);
      writeFileSync(new URL(`${s.id}.svg`, out), `${svg}\n`);
    }
  }
  if (missing.length) console.error(`No icon (the provider's logo is used):\n  ${missing.join('\n  ')}`);
  console.log(`Icons written to ${OUT.pathname}`);
}
