// Extracts the brand logos the app uses from @iconify-json/logos (SVG Logos, CC0) and simple-icons
// (CC0) into a small generated module, so the multi-MB sources never reach the browser bundle. AI
// providers missing from SVG Logos come from @lobehub/icons-static-svg (MIT, see LOBE below). The
// integrations catalog's logos are resolved by integration-logos.mjs.
// Run: npm run logos   (re-run after editing MAP or the catalog)
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { createRequire } from 'node:module';
import { SIMPLE_ICONS, SVG_LOGOS as set, brandIconFile, catalogLogoKeys, cloudIconFile, resolveLogo } from './integration-logos.mjs';

const require = createRequire(import.meta.url);

/** App logo key → SVG Logos icon name, or `si:<slug>` for Simple Icons. Several keys may share one logo. */
const MAP = {
  // notification channels
  teams: 'microsoft-teams',
  'tile:teams': 'microsoft-teams',
  webhook: 'webhooks',
  // data jobs
  spark: 'apache-spark',
  dbt: 'dbt',
  // clouds / platforms
  aws: 'aws',
  amazonwebservices: 'aws',
  azure: 'microsoft-azure',
  'tile:azure': 'microsoft-azure',
  gcp: 'google-cloud',
  googlecloud: 'google-cloud',
  oracle: 'oracle',
  // operating systems / runtimes
  windows: 'microsoft-windows-icon',
  'windows-cert-store': 'microsoft-windows-icon',
  'active-directory': 'microsoft-icon',
  linux: 'linux-tux',
  darwin: 'apple',
  apple: 'apple',
  ubuntu: 'ubuntu',
  android: 'android-icon',
  javascript: 'javascript',
  react: 'react',
  // RUM application types (Manage Applications)
  angular: 'angular-icon',
  vuedotjs: 'vue',
  nextdotjs: 'nextjs-icon',
  nuxt: 'nuxt-icon',
  flutter: 'flutter',
  electron: 'electron',
  unity: 'unity',
  kotlin: 'kotlin-icon',
  salesforce: 'salesforce',
  shopify: 'shopify',
  cplusplus: 'c-plusplus',
  ios: 'ios',
  nodedotjs: 'nodejs-icon',
  python: 'python',
  go: 'go',
  openjdk: 'java',
  dotnet: 'dotnet',
  nvidia: 'nvidia',
  // collectors / containers
  opentelemetry: 'opentelemetry-icon',
  otel: 'opentelemetry-icon',
  kubernetes: 'kubernetes',
  docker: 'docker-icon',
  fastly: 'fastly',
  helm: 'helm',
  // AWS services
  amazonapigateway: 'aws-api-gateway',
  awslambda: 'aws-lambda',
  amazonrds: 'aws-rds',
  amazons3: 'aws-s3',
  amazondynamodb: 'aws-dynamodb',
  amazonec2: 'aws-ec2',
  amazoneks: 'aws-eks',
  amazonecs: 'aws-ecs',
  'tile:aws-athena': 'aws-athena',
  amazoncloudfront: 'aws-cloudfront',
  amazonvpc: 'aws-vpc',
  elasticloadbalancing: 'aws-elb',
  amazoncloudwatch: 'aws-cloudwatch',
  amazonsqs: 'aws-sqs',
  amazonsns: 'aws-sns',
  amazonelasticache: 'aws-elasticache',
  amazonredshift: 'aws-redshift',
  amazonroute53: 'aws-route53',
  awsfargate: 'aws-fargate',
  // Google Cloud services
  googlecloudrun: 'google-cloud-run',
  googlecloudfunctions: 'google-cloud-functions',
  gemini: 'google-gemini',
  // languages
  ruby: 'ruby',
  php: 'php',
  // data stores / messaging / web
  apache: 'apache',
  apachetomcat: 'tomcat',
  redis: 'redis',
  postgresql: 'postgresql',
  mongodb: 'mongodb-icon',
  mysql: 'mysql-icon',
  elasticsearch: 'elasticsearch',
  splunk: 'splunk',
  apachecassandra: 'cassandra',
  'tile:memcached': 'memcached',
  apachekafka: 'kafka-icon',
  rabbitmq: 'rabbitmq-icon',
  nginx: 'nginx',
  envoyproxy: 'envoy-icon',
  gunicorn: 'gunicorn',
  argo: 'argo-icon',
  consul: 'consul',
  etcd: 'etcd',
  vault: 'vault-icon',
  prometheus: 'prometheus',
  snowflake: 'snowflake-icon',
  apacheairflow: 'airflow-icon',
  // AI
  openai: 'openai-icon',
  anthropic: 'anthropic-icon',
  huggingface: 'hugging-face-icon',
  mistral: 'mistral-ai-icon',
  meta: 'meta-icon',
  perplexity: 'perplexity-icon',
  google: 'google-icon',
  nvidia_nim: 'nvidia',
  // devtools / collaboration / security
  github: 'github-icon',
  githubactions: 'github-actions',
  gitlab: 'gitlab',
  jira: 'jira',
  linear: 'linear-icon',
  bitbucket: 'bitbucket',
  confluence: 'confluence',
  notion: 'notion-icon',
  figma: 'figma',
  launchdarkly: 'launchdarkly-icon',
  sonarqube: 'sonarqube',
  googledrive: 'google-drive',
  postman: 'postman-icon',
  miro: 'miro-icon',
  asana: 'asana-icon',
  trello: 'trello',
  terraform: 'terraform-icon',
  ansible: 'ansible',
  jenkins: 'jenkins',
  circleci: 'circleci',
  sentry: 'sentry-icon',
  cypress: 'cypress-icon',
  selenium: 'selenium',
  slack: 'slack-icon',
  'tile:teams': 'microsoft-teams',
  pagerduty: 'pagerduty-icon',
  opsgenie: 'opsgenie',
  zendesk: 'zendesk-icon',
  twilio: 'twilio-icon',
  sendgrid: 'sendgrid-icon',
  stripe: 'stripe',
  cloudflare: 'cloudflare-icon',
  okta: 'okta-icon',
  airtable: 'airtable',
  bamboo: 'bamboo',
  box: 'box',
  zoom: 'zoom-icon',
  // Simple Icons (one colour, in the brand's colour)
  authentik: 'si:authentik',
  backstage: 'si:backstage',
  celery: 'si:celery',
  containerd: 'si:containerd',
  databricks: 'si:databricks',
  fluentbit: 'si:fluentbit',
  istio: 'si:istio',
  ollama: 'si:ollama',
  paloaltonetworks: 'si:paloaltonetworks',
  // Downloaded logos (fetch-brand-icons.mjs)
  netbird: 'brand:netbird',
  fleetdm: 'brand:fleetdm',
};

/** App logo key → LobeHub icon file (MIT), for providers SVG Logos doesn't have. */
const LOBE = {
  deepseek: 'deepseek-color',
  cohere: 'cohere-color',
  bedrock: 'bedrock-color',
  vertexai: 'vertexai-color',
  azureopenai: 'azureai-color',
};

function resolve(name, depth = 0) {
  const icon = set.icons[name];
  if (icon) return icon;
  const alias = set.aliases?.[name];
  if (alias && depth < 3) return { ...resolve(alias.parent, depth + 1), ...alias, parent: undefined };
  return null;
}

/** A generated module's logos: one icon per source reference, shared by every key that shows it. */
const brand = { icons: {}, keys: {} };
const catalog = { icons: {}, keys: {} };
const missing = [];
/** A PNG's width and height (its IHDR chunk). */
const pngSize = (bytes) => [bytes.readUInt32BE(16), bytes.readUInt32BE(20)];

function iconOf(ref) {
  const file = ref.startsWith('cloud:') ? cloudIconFile(ref) : ref.startsWith('brand:') ? brandIconFile(ref) : null;
  if (file?.pathname.endsWith('.png')) {
    const bytes = readFileSync(file);
    const [width, height] = pngSize(bytes);
    return { body: `<image width="${width}" height="${height}" href="data:image/png;base64,${bytes.toString('base64')}"/>`, width, height };
  }
  if (file) {
    const svg = readFileSync(file, 'utf8');
    const [, x, y, w, h] = /viewBox="(\S+)[\s,]+(\S+)[\s,]+(\S+)[\s,]+(\S+)"/.exec(svg) ?? [, 0, 0, /\swidth="([\d.]+)/.exec(svg)?.[1], /\sheight="([\d.]+)/.exec(svg)?.[1]];
    const inner = svg
      .replace(/^[\s\S]*?<svg[^>]*>/, '')
      .replace(/<\/svg>\s*$/, '')
      .replace(/\s*\n\s*/g, ' ');
    // A viewBox not at the origin: shift the drawing so the logo's box starts at 0 0.
    const body = Number(x) || Number(y) ? `<g transform="translate(${-x} ${-y})">${inner}</g>` : inner;
    return { body, width: Number(w), height: Number(h) };
  }
  if (ref.startsWith('si:')) {
    const icon = SIMPLE_ICONS.get(ref.slice(3));
    return icon && { body: `<path fill="#${icon.hex}" d="${icon.path}"/>`, width: 24, height: 24 };
  }
  const icon = resolve(ref.replace(/^logos:/, ''));
  return icon && { body: icon.body, width: icon.width ?? set.width ?? 256, height: icon.height ?? set.height ?? 256 };
}
function add(group, key, ref) {
  const icon = group.icons[ref] ?? brand.icons[ref] ?? iconOf(ref);
  if (!icon) return missing.push(`${key} → ${ref}`);
  group.icons[ref] = icon;
  group.keys[key] = ref;
}
for (const [key, name] of Object.entries(MAP)) add(brand, key, /^(si|brand):/.test(name) ? name : `logos:${name}`);
const lobeDir = dirname(require.resolve('@lobehub/icons-static-svg/package.json'));
for (const [key, name] of Object.entries(LOBE)) {
  let svg;
  try {
    svg = readFileSync(`${lobeDir}/icons/${name}.svg`, 'utf8');
  } catch {
    missing.push(`${key} → ${name}.svg`);
    continue;
  }
  const [, , , w, h] = /viewBox="(\S+) (\S+) (\S+) (\S+)"/.exec(svg) ?? [];
  const body = svg
    .replace(/^<svg[^>]*>/, '')
    .replace(/<\/svg>\s*$/, '')
    .replace(/<title>[^<]*<\/title>/, '');
  brand.icons[`lobe:${name}`] = { body, width: Number(w) || 24, height: Number(h) || 24 };
  brand.keys[key] = `lobe:${name}`;
}
// The integrations catalog's logos, served as files (public/images/integrations/logos) the tiles load lazily.
for (const entry of catalogLogoKeys()) {
  const ref = resolveLogo(entry);
  // Curated keys show their brand logo; the catalog's own reference still gets a file (dashboards' images use it).
  const curated = brand.keys[entry.key];
  if (curated) add(catalog, entry.key, curated);
  else if (ref) add(catalog, entry.key, ref);
}
if (missing.length) {
  console.error(`Missing logos:\n  ${missing.join('\n  ')}`);
  process.exit(1);
}

const header = `// GENERATED by scripts/build-logos.mjs from @iconify-json/logos (SVG Logos, CC0-1.0), simple-icons
// (Simple Icons, CC0-1.0) and @lobehub/icons-static-svg (LobeHub Icons, MIT, Copyright (c) 2023 LobeHub). Do not edit.
// Brand names and logos are trademarks of their owners; shown only to identify the technology.
`;
const sorted = (o) => Object.fromEntries(Object.entries(o).sort(([a], [b]) => a.localeCompare(b)));
function write(name, exportName, { icons, keys }) {
  const file = new URL(`../src/assets/${name}`, import.meta.url);
  const byKey = Object.entries(sorted(keys))
    .map(([key, ref]) => `  ${JSON.stringify(key)}: ICONS[${JSON.stringify(ref)}],`)
    .join('\n');
  writeFileSync(
    file,
    `${header}type Logo = { body: string; width: number; height: number };\nconst ICONS: Record<string, Logo> = ${JSON.stringify(sorted(icons), null, 1)};\n\nexport const ${exportName}: Record<string, Logo> = {\n${byKey}\n};\n`,
  );
  console.log(`${name}: ${Object.keys(keys).length} keys, ${Object.keys(icons).length} logos (${Math.round(readFileSync(file).length / 1024)} KB).`);
}
write('brandLogos.generated.ts', 'BRAND_LOGOS', brand);

/** One file per catalog logo and a manifest of key → file, so a page loads only the logos it shows. */
function writeCatalog({ keys }) {
  const dir = new URL('../public/images/integrations/logos/', import.meta.url);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const files = {};
  for (const ref of new Set(Object.values(keys))) {
    const slug = ref.replace(/[^a-z0-9]+/gi, '-').toLowerCase();
    const source = ref.startsWith('brand:') ? brandIconFile(ref) : null;
    if (source?.pathname.endsWith('.png')) {
      copyFileSync(source, new URL(`${slug}.png`, dir));
      files[ref] = `${slug}.png`;
      continue;
    }
    const { body, width, height } = catalog.icons[ref];
    writeFileSync(new URL(`${slug}.svg`, dir), `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 ${width} ${height}">${body}</svg>\n`);
    files[ref] = `${slug}.svg`;
  }
  const file = new URL('../src/assets/integrationLogos.generated.ts', import.meta.url);
  const byKey = Object.entries(sorted(keys))
    .map(([key, ref]) => `  ${JSON.stringify(key)}: ${JSON.stringify(files[ref])},`)
    .join('\n');
  writeFileSync(
    file,
    `${header}// The cloud providers' service icons and the downloaded logos keep their own terms: see src/assets/{cloud,brand}-icons/NOTICE.md.\n\n/** Catalog logo key → its file in /images/integrations/logos/. */\nexport const INTEGRATION_LOGOS: Record<string, string> = {\n${byKey}\n};\n`,
  );
  console.log(`integrationLogos.generated.ts: ${Object.keys(keys).length} keys, ${Object.keys(files).length} files.`);
}
writeCatalog(catalog);
