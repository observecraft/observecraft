// Logo registry for integrations/apps. Order: full-colour SVG Logos (generated subset, CC0) →
// single-colour Simple Icons in the brand colour (CC0) → wordmark / coloured tile.
// Brands remain trademarks of their owners and are shown only to identify the integration.
import {
  siAirtable,
  siAmazonapigateway,
  siAmazondynamodb,
  siAmazonec2,
  siAmazonecs,
  siAmazoneks,
  siAmazonrds,
  siAmazons3,
  siAmazonwebservices,
  siAnsible,
  siAnthropic,
  siApache,
  siApacheairflow,
  siApachecassandra,
  siApachekafka,
  siApachetomcat,
  siArgo,
  siAwslambda,
  siBamboo,
  siBox,
  siCelery,
  siCircleci,
  siCloudflare,
  siConsul,
  siContainerd,
  siCypress,
  siDatabricks,
  siDocker,
  siDotnet,
  siElasticsearch,
  siEnvoyproxy,
  siEtcd,
  siFluentbit,
  siGithub,
  siGithubactions,
  siGitlab,
  siGo,
  siGooglecloud,
  siGunicorn,
  siHelm,
  siHuggingface,
  siIstio,
  siJenkins,
  siJira,
  siKubernetes,
  siLinux,
  siMongodb,
  siMysql,
  siNginx,
  siNodedotjs,
  siOkta,
  siOllama,
  siOpenai,
  siOpenjdk,
  siOpentelemetry,
  siOpsgenie,
  siOracle,
  siPagerduty,
  siPaloaltonetworks,
  siPostgresql,
  siPrometheus,
  siPython,
  siRabbitmq,
  siRedis,
  siSelenium,
  siSentry,
  siSlack,
  siSnowflake,
  siStripe,
  siTerraform,
  siTwilio,
  siVault,
  siZendesk,
  siZoom,
  type SimpleIcon,
} from 'simple-icons';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';

const ICONS: Record<string, SimpleIcon> = {
  airtable: siAirtable,
  amazonapigateway: siAmazonapigateway,
  amazondynamodb: siAmazondynamodb,
  amazonec2: siAmazonec2,
  amazonecs: siAmazonecs,
  amazoneks: siAmazoneks,
  amazonrds: siAmazonrds,
  amazons3: siAmazons3,
  amazonwebservices: siAmazonwebservices,
  ansible: siAnsible,
  anthropic: siAnthropic,
  apache: siApache,
  apacheairflow: siApacheairflow,
  apachecassandra: siApachecassandra,
  apachekafka: siApachekafka,
  apachetomcat: siApachetomcat,
  argo: siArgo,
  awslambda: siAwslambda,
  bamboo: siBamboo,
  box: siBox,
  celery: siCelery,
  circleci: siCircleci,
  cloudflare: siCloudflare,
  consul: siConsul,
  containerd: siContainerd,
  cypress: siCypress,
  databricks: siDatabricks,
  docker: siDocker,
  dotnet: siDotnet,
  elasticsearch: siElasticsearch,
  envoyproxy: siEnvoyproxy,
  etcd: siEtcd,
  fluentbit: siFluentbit,
  github: siGithub,
  githubactions: siGithubactions,
  gitlab: siGitlab,
  go: siGo,
  googlecloud: siGooglecloud,
  gunicorn: siGunicorn,
  helm: siHelm,
  huggingface: siHuggingface,
  istio: siIstio,
  jenkins: siJenkins,
  jira: siJira,
  kubernetes: siKubernetes,
  linux: siLinux,
  mongodb: siMongodb,
  mysql: siMysql,
  nginx: siNginx,
  nodedotjs: siNodedotjs,
  okta: siOkta,
  ollama: siOllama,
  openai: siOpenai,
  openjdk: siOpenjdk,
  opentelemetry: siOpentelemetry,
  opsgenie: siOpsgenie,
  oracle: siOracle,
  pagerduty: siPagerduty,
  paloaltonetworks: siPaloaltonetworks,
  postgresql: siPostgresql,
  prometheus: siPrometheus,
  python: siPython,
  rabbitmq: siRabbitmq,
  redis: siRedis,
  selenium: siSelenium,
  sentry: siSentry,
  slack: siSlack,
  snowflake: siSnowflake,
  stripe: siStripe,
  terraform: siTerraform,
  twilio: siTwilio,
  vault: siVault,
  zendesk: siZendesk,
  zoom: siZoom,
};

/** Fallbacks: "word" renders a coloured wordmark, "tile" a coloured square with short text. */
const FALLBACK: Record<string, { text: string; color: string; style: 'word' | 'tile' }> = {
  keda: { text: 'KEDA', color: '#326ce5', style: 'word' },
  velero: { text: 'VELERO', color: '#2c7fb8', style: 'word' },
  coredns: { text: 'CoreDNS', color: '#7b42bc', style: 'word' },
  supervisord: { text: 'SUPERVISORD', color: '#3b3b3b', style: 'word' },
  'windows-cert-store': { text: 'Cert Store', color: '#0078d4', style: 'word' },
  'active-directory': { text: 'Active Directory', color: '#0078d4', style: 'word' },
  'tile:aws-appstream': { text: 'AS', color: '#01a88d', style: 'tile' },
  'tile:aws-athena': { text: 'Ath', color: '#8c4fff', style: 'tile' },
  'tile:aws-bedrock': { text: 'Br', color: '#01a88d', style: 'tile' },
  'tile:azure': { text: 'Az', color: '#0078d4', style: 'tile' },
  'tile:teams': { text: 'T', color: '#5059c9', style: 'tile' },
  'tile:memcached': { text: 'Mc', color: '#2a7d4f', style: 'tile' },
  'tile:haproxy': { text: 'HA', color: '#106da9', style: 'tile' },
};

function initials(text: string): string {
  return text
    .split(/[\s-]+/)
    .map((w) => w[0])
    .join('')
    .slice(0, 3)
    .toUpperCase();
}

/** Deterministic colour for unknown keys. */
function hashColor(key: string): string {
  const palette = ['#5b5fc7', '#d6336c', '#1c7ed6', '#0ca678', '#f08c00', '#7048e8', '#e8590c'];
  let h = 0;
  for (const c of key) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return palette[h % palette.length];
}

interface Props {
  logo: string;
  name: string;
  size?: number;
}

export function IntegrationLogo({ logo, name, size = 64 }: Props) {
  if (hasBrandLogo(logo)) return <BrandLogo name={logo} size={size} title={name} maxRatio={2.8} />;
  const icon = ICONS[logo];
  if (icon) {
    return (
      <svg role="img" aria-label={name} viewBox="0 0 24 24" width={size} height={size} fill={`#${icon.hex}`}>
        <path d={icon.path} />
      </svg>
    );
  }
  const fb = FALLBACK[logo] ?? { text: logo.startsWith('tile:') ? initials(name) : name, color: hashColor(logo), style: logo.startsWith('tile:') ? 'tile' : 'word' };
  if (fb.style === 'tile') {
    return (
      <span className="logo-tile" style={{ width: size * 1.25, height: size * 1.25, background: fb.color, fontSize: size * 0.42 }} aria-label={name}>
        {fb.text}
      </span>
    );
  }
  return (
    <span className="logo-word" style={{ color: fb.color, fontSize: Math.max(14, size * (fb.text.length > 8 ? 0.36 : 0.62)) }} aria-label={name}>
      {fb.text}
    </span>
  );
}
