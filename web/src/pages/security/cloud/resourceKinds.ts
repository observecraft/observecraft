// How a finding's resource type reads in Cloud Security: a name (EC2 Security Group, DaemonSet, Virtual Machine) and the
// icon of its service (the AWS, Azure or Google Cloud service icon, the Kubernetes kind's icon, the host). Types are the
// scanner's ASFF names (AwsEc2SecurityGroup), Azure resource types (Microsoft.Compute/virtualMachines), Google Cloud asset
// types (compute.googleapis.com/Instance), our rules' kube_* kinds and host / container. Pure: scripts/resource-kinds.test.mjs.
import { BRAND_LOGOS } from '../../../assets/brandLogos.generated';
import { INTEGRATION_LOGOS } from '../../../assets/integrationLogos.generated';

/** Kubernetes kinds as Kubernetes spells them. */
const KUBE: Record<string, string> = {
  apiserver: 'API Server',
  cluster: 'Cluster',
  clusterrole: 'ClusterRole',
  clusterrolebinding: 'ClusterRoleBinding',
  configmap: 'ConfigMap',
  cronjob: 'CronJob',
  daemonset: 'DaemonSet',
  deployment: 'Deployment',
  ingress: 'Ingress',
  job: 'Job',
  namespace: 'Namespace',
  networkpolicy: 'NetworkPolicy',
  node: 'Node',
  persistentvolume: 'PersistentVolume',
  persistentvolumeclaim: 'PersistentVolumeClaim',
  pod: 'Pod',
  replicaset: 'ReplicaSet',
  role: 'Role',
  rolebinding: 'RoleBinding',
  secret: 'Secret',
  service: 'Service',
  serviceaccount: 'ServiceAccount',
  statefulset: 'StatefulSet',
};

/** AWS services in ASFF type names, longest first: their name and their icon (an integrations logo key). */
const AWS: [prefix: string, name: string, logo: string | null][] = (
  [
    ['ApiGatewayV2', 'API Gateway', 'tile:aws-apigateway'],
    ['ApiGateway', 'API Gateway', 'tile:aws-apigateway'],
    ['AutoScaling', 'Auto Scaling', 'tile:aws-autoscaling'],
    ['Backup', 'Backup', 'tile:aws-backup'],
    ['CertificateManager', 'Certificate Manager', 'tile:aws-certificatemanager'],
    ['CloudFront', 'CloudFront', 'tile:aws-cloudfront'],
    ['CloudTrail', 'CloudTrail', 'tile:aws-cloudtrail'],
    ['CloudWatch', 'CloudWatch', 'tile:aws-logs'],
    ['CodeBuild', 'CodeBuild', 'tile:aws-codebuild'],
    ['Cognito', 'Cognito', 'tile:aws-cognito'],
    ['DynamoDb', 'DynamoDB', 'tile:aws-dynamodb'],
    ['Ec2', 'EC2', 'tile:aws-ec2'],
    ['Ecr', 'ECR', 'tile:aws-ecr'],
    ['Ecs', 'ECS', 'tile:aws-ecs'],
    ['Efs', 'EFS', 'tile:aws-efs'],
    ['Eks', 'EKS', 'kubernetes'],
    ['ElastiCache', 'ElastiCache', 'tile:aws-elasticache'],
    ['ElasticBeanstalk', 'Elastic Beanstalk', 'tile:aws-elasticbeanstalk'],
    ['Elasticsearch', 'OpenSearch', 'tile:aws-es'],
    ['OpenSearch', 'OpenSearch', 'tile:aws-es'],
    ['ElbV2', 'ELB', 'tile:aws-applicationelb'],
    ['Elb', 'ELB', 'tile:aws-elb'],
    ['EventBridge', 'EventBridge', 'tile:aws-events'],
    ['Glue', 'Glue', 'tile:aws-glue'],
    ['Iam', 'IAM', 'iam-access-analyzer'],
    ['Kinesis', 'Kinesis', 'tile:aws-kinesis'],
    ['Kms', 'KMS', 'tile:aws-kms'],
    ['Lambda', 'Lambda', 'tile:aws-lambda'],
    ['Msk', 'MSK', 'tile:aws-kafka'],
    ['NetworkFirewall', 'Network Firewall', 'tile:aws-networkfirewall'],
    ['Rds', 'RDS', 'tile:aws-rds'],
    ['Redshift', 'Redshift', 'tile:aws-redshift'],
    ['Route53', 'Route 53', 'tile:aws-route53'],
    ['S3', 'S3', 'tile:aws-s3'],
    ['SageMaker', 'SageMaker', 'tile:aws-sagemaker'],
    ['SecretsManager', 'Secrets Manager', 'tile:aws-secretsmanager'],
    ['Sns', 'SNS', 'tile:aws-sns'],
    ['Sqs', 'SQS', 'tile:aws-sqs'],
    ['Ssm', 'Systems Manager', 'tile:aws-ssm'],
    ['StepFunctions', 'Step Functions', 'tile:aws-states'],
    ['WafV2', 'WAF', 'tile:aws-wafv2'],
    ['Waf', 'WAF', 'tile:aws-waf'],
  ] as [string, string, string | null][]
).sort((a, b) => b[0].length - a[0].length);

/** Words of a CamelCase name, acronyms kept (DbInstance → DB Instance). */
function words(s: string): string {
  return s
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .replace(/\bDb\b/g, 'DB')
    .replace(/\bAcl\b/g, 'ACL')
    .replace(/\bVpc\b/g, 'VPC')
    .replace(/\bIp\b/g, 'IP')
    .trim();
}

export type Kind = { name: string; logo: string | null; fallback?: 'host' | 'container' | 'aws' | 'azure' | 'gcp' | 'repository'; kube?: string };

/** Google Cloud services in Cloud Asset types (`compute.googleapis.com/Instance`): their name and icon id. */
const GCP: Record<string, [name: string, icon: string]> = {
  compute: ['Compute Engine', 'gce'],
  storage: ['Cloud Storage', 'gcs'],
  container: ['GKE', 'gke'],
  sqladmin: ['Cloud SQL', 'cloudsql'],
  cloudfunctions: ['Cloud Functions', 'cloudfunctions'],
  run: ['Cloud Run', 'run'],
  pubsub: ['Pub/Sub', 'pubsub'],
  bigquery: ['BigQuery', 'bigquery'],
  iam: ['IAM', 'iam'],
  cloudkms: ['Cloud KMS', 'cloudkms'],
  dns: ['Cloud DNS', 'dns'],
  redis: ['Memorystore', 'redis'],
  artifactregistry: ['Artifact Registry', 'artifactregistry'],
  logging: ['Cloud Logging', 'logging'],
};

const hasLogo = (key: string) => key in BRAND_LOGOS || key in INTEGRATION_LOGOS;
const singular = (s: string) => (/(ss|us)$/.test(s) ? s : s.replace(/ies$/, 'y').replace(/s$/, ''));

/** An Azure resource type (`Microsoft.Compute/virtualMachines`): its service icon, else Azure's logo. */
function azureKind(ns: string, path: string): Kind {
  const segments = path.split('/').filter(Boolean);
  const last = singular(segments[segments.length - 1] ?? '');
  const service = words(ns);
  let name = words(last.charAt(0).toUpperCase() + last.slice(1));
  if (service.endsWith(name)) name = service;
  for (let n = segments.length; n > 0; n--) {
    const key = `tile:azure-${ns.toLowerCase()}-${segments.slice(0, n).join('-').toLowerCase()}`;
    if (hasLogo(key)) return { name, logo: key, fallback: 'azure' };
  }
  return { name, logo: 'azure', fallback: 'azure' };
}

/** A repository's host logo (GitHub, GitLab, Bitbucket) from its name or URL. */
function repositoryKind(resource: string): Kind {
  const host = /(github|gitlab|bitbucket)/i.exec(resource)?.[1]?.toLowerCase() ?? null;
  return { name: 'Repository', logo: host, fallback: 'repository' };
}

/** How a resource type reads; `resource` (its name or URL) tells a repository's host. */
export function resourceKind(type: string, resource = ''): Kind {
  const kube = /^kube_(.+)$/.exec(type);
  if (kube) return { name: KUBE[kube[1]] ?? words(kube[1].charAt(0).toUpperCase() + kube[1].slice(1)), logo: 'kubernetes', kube: kube[1] };
  if (type === 'host') return { name: 'Host', logo: null, fallback: 'host' };
  if (type === 'container') return { name: 'Container', logo: 'docker' };
  if (type === 'ecs_service') return { name: 'ECS Service', logo: 'tile:aws-ecs' };
  if (type === 'Repository') return repositoryKind(resource);
  const azure = /^microsoft\.([^/]+)\/(.+)$/i.exec(type);
  if (azure) return azureKind(azure[1], azure[2]);
  const gcp = /^([a-z0-9]+)\.googleapis\.com\/(.+)$/.exec(type);
  if (gcp) {
    const svc = GCP[gcp[1]];
    const name = words(gcp[2].split('/').pop() ?? gcp[2]);
    if (svc && hasLogo(`tile:gcp-${svc[1]}`)) return { name: `${svc[0]} ${name}`, logo: `tile:gcp-${svc[1]}`, fallback: 'gcp' };
    return { name: svc ? `${svc[0]} ${name}` : name, logo: 'googlecloud', fallback: 'gcp' };
  }
  const aws = /^Aws(.+)$/.exec(type);
  if (aws) {
    if (aws[1] === 'Account') return { name: 'AWS Account', logo: 'aws' };
    const svc = AWS.find(([p]) => aws[1].startsWith(p) && /^[A-Z0-9]|^$/.test(aws[1].slice(p.length)));
    if (svc) return { name: `${svc[1]} ${words(aws[1].slice(svc[0].length))}`.trim(), logo: svc[2], fallback: 'aws' };
    return { name: words(aws[1]), logo: 'aws' };
  }
  return { name: type, logo: null };
}

/** Distributions as `os_of` names them (`debian 12`, `amazon linux 2023`), first match: their logo key. */
const OS_LOGOS: [prefix: string, logo: string][] = [
  ['debian', 'debian'],
  ['ubuntu', 'ubuntu'],
  ['alpine', 'alpine'],
  ['red hat', 'redhat'],
  ['redhat', 'redhat'],
  ['rhel', 'redhat'],
  ['centos', 'centos'],
  ['amazon', 'aws'],
  ['rocky', 'rockylinux'],
  ['alma', 'almalinux'],
  ['fedora', 'fedora'],
  ['oracle', 'oracle'],
  ['opensuse', 'opensuse'],
  ['suse', 'suse'],
  ['wolfi', 'chainguard'],
  ['chainguard', 'chainguard'],
  ['azure', 'azure'],
];

/** The logo of a host's or image's distribution; another Linux gets Tux, none gets none. */
export function osLogo(os: string): string | null {
  const s = os.trim().toLowerCase();
  if (!s) return null;
  return OS_LOGOS.find(([p]) => s.startsWith(p))?.[1] ?? 'linux';
}

const ECOSYSTEM_LOGOS: Record<string, string> = { npm: 'npm', Maven: 'maven', PyPI: 'pypi', RubyGems: 'rubygems', 'crates.io': 'rust', NuGet: 'nuget', Packagist: 'composer', Go: 'go' };

/** The logo of a package's ecosystem: the language registry's, or an OS package's distribution. */
export function ecosystemLogo(ecosystem: string): string | null {
  if (ECOSYSTEM_LOGOS[ecosystem]) return ECOSYSTEM_LOGOS[ecosystem];
  const base = ecosystem.split(':')[0];
  return ecosystem.includes(':') || OS_LOGOS.some(([p]) => base.toLowerCase().startsWith(p)) ? osLogo(base) : null;
}
