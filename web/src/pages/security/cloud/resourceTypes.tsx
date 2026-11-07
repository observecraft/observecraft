// How a finding's resource type reads in Cloud Security: a name (EC2 Security Group, DaemonSet, IAM User) and the icon
// of its service (the AWS service icon, Kubernetes, the host). Types are the scanner's ASFF names (AwsEc2SecurityGroup),
// our rules' kube_* kinds and host / container.
import { Box, Server } from 'lucide-react';
import { INTEGRATION_LOGOS } from '../../../assets/integrationLogos.generated';
import { BrandLogo, hasBrandLogo } from '../../../components/brand/BrandLogo';

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
    ['CloudTrail', 'CloudTrail', null],
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
    ['SecretsManager', 'Secrets Manager', null],
    ['Sns', 'SNS', 'tile:aws-sns'],
    ['Sqs', 'SQS', 'tile:aws-sqs'],
    ['Ssm', 'Systems Manager', null],
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

type Kind = { name: string; logo: string | null; fallback?: 'host' | 'container' | 'aws' };

export function resourceKind(type: string): Kind {
  const kube = /^kube_(.+)$/.exec(type);
  if (kube) return { name: KUBE[kube[1]] ?? words(kube[1].charAt(0).toUpperCase() + kube[1].slice(1)), logo: 'kubernetes' };
  if (type === 'host') return { name: 'Host', logo: null, fallback: 'host' };
  if (type === 'container') return { name: 'Container', logo: 'docker' };
  if (type === 'ecs_service') return { name: 'ECS Service', logo: 'tile:aws-ecs' };
  if (type === 'Repository') return { name: 'Repository', logo: 'github' };
  const aws = /^Aws(.+)$/.exec(type);
  if (aws) {
    if (aws[1] === 'Account') return { name: 'AWS Account', logo: 'aws' };
    const svc = AWS.find(([p]) => aws[1].startsWith(p) && /^[A-Z0-9]|^$/.test(aws[1].slice(p.length)));
    if (svc) return { name: `${svc[1]} ${words(aws[1].slice(svc[0].length))}`.trim(), logo: svc[2], fallback: 'aws' };
    return { name: words(aws[1]), logo: 'aws' };
  }
  return { name: type, logo: null };
}

function KindIcon({ kind, size }: { kind: Kind; size: number }) {
  const key = kind.logo;
  if (key && hasBrandLogo(key)) return <BrandLogo name={key} size={size} title={kind.name} maxRatio={1.7} />;
  if (key && key in INTEGRATION_LOGOS) return <img className="logo-file" src={`/images/integrations/logos/${INTEGRATION_LOGOS[key]}`} alt="" loading="lazy" width={size} height={size} />;
  if (kind.fallback === 'aws' || key === 'aws') return <BrandLogo name="aws" size={size} title={kind.name} maxRatio={1.7} />;
  if (kind.fallback === 'host') return <Server size={size} />;
  return <Box size={size} />;
}

/** The icon of a resource type alone (facet values). */
export function ResourceTypeIcon({ type, size = 14 }: { type: string; size?: number }) {
  return (
    <span className="csm-rtype-icon" style={{ minWidth: size, height: size }}>
      <KindIcon kind={resourceKind(type)} size={size} />
    </span>
  );
}

/** A resource type with its icon: EC2 Security Group, IAM User, DaemonSet. */
export function ResourceType({ type, size = 16 }: { type: string; size?: number }) {
  const kind = resourceKind(type);
  return (
    <span className="csm-rtype" title={type}>
      <span className="csm-rtype-icon" style={{ minWidth: size, height: size }}>
        <KindIcon kind={kind} size={size} />
      </span>
      {kind.name}
    </span>
  );
}
