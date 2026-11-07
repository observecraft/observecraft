// In-browser Cloud Security account scans (mock mode): the demo AWS account's latest Prowler scan and its findings, as
// the server serves them (routes/cloud_posture.rs; findings merged into cloud_security::misconfigurations with
// `posture:<id>` keys). Resources and values are the demo's; check ids follow Prowler's naming.
import type { MisconfigFinding, MisconfigRuleStatus, PostureScans } from '../cloudSecurity/types';

const HOUR = 3_600_000;
const now = Date.now();
const ACCOUNT = '123456789012';
const ACCOUNT_REF = '5b6c7d8e-9f01-4a2b-8c3d-4e5f6a7b8c9d';

interface Check {
  id: string;
  name: string;
  severity: string;
  description: string;
  remediation: string;
  frameworks: string[];
  resourceType: string;
  evaluated: number;
  /** [resource name, resource id, region, detail, team] */
  fails: [string, string, string, string, string][];
}

const CHECKS: Check[] = [
  {
    id: 's3_bucket_level_public_access_block',
    name: 'S3 buckets block public access',
    severity: 'high',
    description: 'Checks that each bucket turns on all four Block Public Access settings.',
    remediation: 'Turn on Block Public Access for the bucket, or for the whole account.',
    frameworks: ['CIS-3.0', 'SOC2', 'PCI-4.0'],
    resourceType: 'AwsS3Bucket',
    evaluated: 14,
    fails: [['example-shop-assets', 'arn:aws:s3:::example-shop-assets', 'eu-central-1', 'S3 Bucket example-shop-assets allows public ACLs.', 'storefront']],
  },
  {
    id: 'rds_instance_storage_encrypted',
    name: 'RDS instances encrypt their storage',
    severity: 'medium',
    description: 'Checks that RDS instances store their data encrypted at rest.',
    remediation: 'Restore the instance from an encrypted snapshot copy; encryption can only be chosen at creation.',
    frameworks: ['CIS-3.0', 'HIPAA', 'SOC2'],
    resourceType: 'AwsRdsDbInstance',
    evaluated: 4,
    fails: [['inventory-replica', 'arn:aws:rds:eu-central-1:123456789012:db:inventory-replica', 'eu-central-1', 'RDS Instance inventory-replica is not encrypted.', 'platform']],
  },
  {
    id: 'ec2_securitygroup_allow_ingress_from_internet_to_tcp_port_22',
    name: 'No security group allows SSH from the internet',
    severity: 'high',
    description: 'Checks for security groups that open TCP 22 to 0.0.0.0/0 or ::/0.',
    remediation: 'Remove the rule, or limit it to known addresses; prefer Session Manager to SSH.',
    frameworks: ['CIS-3.0', 'PCI-4.0'],
    resourceType: 'AwsEc2SecurityGroup',
    evaluated: 23,
    fails: [
      ['bastion-sg', 'arn:aws:ec2:eu-central-1:123456789012:security-group/sg-0a1b2c3d4e5f60001', 'eu-central-1', 'Security group bastion-sg allows TCP 22 from 0.0.0.0/0.', 'platform'],
      ['ci-runners-sg', 'arn:aws:ec2:eu-west-1:123456789012:security-group/sg-0a1b2c3d4e5f60002', 'eu-west-1', 'Security group ci-runners-sg allows TCP 22 from 0.0.0.0/0.', 'platform'],
    ],
  },
  {
    id: 'iam_user_mfa_enabled_console_access',
    name: 'Console users have MFA',
    severity: 'high',
    description: 'Checks that every IAM user with a console password has an MFA device.',
    remediation: 'Assign an MFA device to the user, or remove its console password.',
    frameworks: ['CIS-3.0', 'SOC2'],
    resourceType: 'AwsIamUser',
    evaluated: 6,
    fails: [['release-bot', 'arn:aws:iam::123456789012:user/release-bot', 'us-east-1', 'User release-bot has console access without MFA.', 'platform']],
  },
  {
    id: 'cloudtrail_multi_region_enabled',
    name: 'CloudTrail records every region',
    severity: 'high',
    description: 'Checks for a trail logging management events in all regions.',
    remediation: 'Create a multi-region trail, or turn on multi-region logging on an existing one.',
    frameworks: ['CIS-3.0', 'SOC2', 'PCI-4.0'],
    resourceType: 'AwsCloudTrailTrail',
    evaluated: 1,
    fails: [],
  },
  {
    id: 'ec2_ebs_default_encryption',
    name: 'EBS encryption by default is on',
    severity: 'medium',
    description: 'Checks that new EBS volumes of each region are encrypted by default.',
    remediation: 'Turn on EBS encryption by default in each region.',
    frameworks: ['CIS-3.0'],
    resourceType: 'AwsAccount',
    evaluated: 3,
    fails: [['eu-west-1', 'arn:aws:ec2:eu-west-1:123456789012:account', 'eu-west-1', 'EBS default encryption is not enabled in eu-west-1.', 'platform']],
  },
];

export function postureFindings(triage: Map<string, string>): MisconfigFinding[] {
  return CHECKS.flatMap((c) =>
    c.fails.map(([name, id, region, detail, team], i) => {
      const key = `posture:00000000-0000-4000-8000-${String(CHECKS.indexOf(c) * 10 + i).padStart(12, '0')}`;
      return {
        key,
        ruleId: c.id,
        ruleName: c.name,
        severity: c.severity,
        description: c.description,
        remediation: c.remediation,
        frameworks: c.frameworks,
        detail,
        resourceId: id,
        resourceName: name,
        resourceType: c.resourceType,
        cluster: null,
        namespace: null,
        account: ACCOUNT,
        region,
        env: 'prod',
        service: null,
        team,
        status: triage.get(key) ?? 'open',
        lastSeen: now - 3 * HOUR,
      };
    }),
  );
}

export const postureRules = (): MisconfigRuleStatus[] =>
  CHECKS.map((c) => ({
    id: c.id,
    name: c.name,
    severity: c.severity,
    description: c.description,
    remediation: c.remediation,
    frameworks: c.frameworks,
    evaluated: c.evaluated,
    failing: c.fails.length,
  }));

let scan: PostureScans['accounts'][number] = {
  provider: 'aws',
  accountRef: ACCOUNT_REF,
  accountId: ACCOUNT,
  enabled: true,
  status: 'done',
  queuedAt: now - 3 * HOUR - 600_000,
  startedAt: now - 3 * HOUR - 540_000,
  finishedAt: now - 3 * HOUR,
  error: null,
  toolVersion: '5.47.0',
  passed: CHECKS.reduce((n, c) => n + c.evaluated - c.fails.length, 0),
  failed: CHECKS.reduce((n, c) => n + c.fails.length, 0),
};

export const postureScans = (): Promise<PostureScans> => new Promise((r) => setTimeout(() => r({ scannerConfigured: true, intervalHours: 24, accounts: [{ ...scan }] }), 120));

export function scanAccountNow(accountRef: string): Promise<void> {
  if (accountRef !== ACCOUNT_REF) return Promise.reject(new Error('No AWS account with Cloud Security scanning turned on'));
  scan = { ...scan, status: 'queued', queuedAt: Date.now(), startedAt: null, finishedAt: null };
  return new Promise((r) => setTimeout(r, 150));
}
