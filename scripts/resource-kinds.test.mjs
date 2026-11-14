import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
});

const { resourceKind } = await import('../web/src/pages/security/cloud/resourceKinds.ts');

test('AWS types: CloudTrail, Secrets Manager and Systems Manager have their service icons', () => {
  assert.deepEqual(resourceKind('AwsCloudTrailTrail'), { name: 'CloudTrail Trail', logo: 'tile:aws-cloudtrail', fallback: 'aws' });
  assert.equal(resourceKind('AwsSecretsManagerSecret').logo, 'tile:aws-secretsmanager');
  assert.equal(resourceKind('AwsSsmPatchCompliance').logo, 'tile:aws-ssm');
  assert.equal(resourceKind('AwsEc2SecurityGroup').name, 'EC2 Security Group');
});

test('Azure resource types get the service icon, else the Azure logo', () => {
  assert.deepEqual(resourceKind('Microsoft.Compute/virtualMachines'), { name: 'Virtual Machine', logo: 'tile:azure-compute-virtualmachines', fallback: 'azure' });
  assert.equal(resourceKind('microsoft.storage/storageAccounts').logo, 'tile:azure-storage-storageaccounts');
  assert.equal(resourceKind('Microsoft.KeyVault/vaults').name, 'Key Vault');
  assert.deepEqual(resourceKind('Microsoft.Unknown/widgets'), { name: 'Widget', logo: 'azure', fallback: 'azure' });
});

test('Google Cloud asset types get the service icon, else the Google Cloud logo', () => {
  assert.deepEqual(resourceKind('compute.googleapis.com/Instance'), { name: 'Compute Engine Instance', logo: 'tile:gcp-gce', fallback: 'gcp' });
  assert.equal(resourceKind('storage.googleapis.com/Bucket').logo, 'tile:gcp-gcs');
  assert.equal(resourceKind('container.googleapis.com/Cluster').logo, 'tile:gcp-gke');
  assert.equal(resourceKind('sqladmin.googleapis.com/Instance').logo, 'tile:gcp-cloudsql');
  assert.equal(resourceKind('example.googleapis.com/Thing').logo, 'googlecloud');
});

test('repositories show their host, not always GitHub', () => {
  assert.equal(resourceKind('Repository', 'github.com/example-org/storefront').logo, 'github');
  assert.equal(resourceKind('Repository', 'https://gitlab.example.com/platform/checkout').logo, 'gitlab');
  assert.equal(resourceKind('Repository', 'bitbucket.org/example/api').logo, 'bitbucket');
  assert.deepEqual(resourceKind('Repository', 'git.example.com/team/app'), { name: 'Repository', logo: null, fallback: 'repository' });
  assert.equal(resourceKind('Repository').logo, null);
});

test('Kubernetes, hosts and containers keep their kinds', () => {
  assert.equal(resourceKind('kube_daemonset').name, 'DaemonSet');
  assert.equal(resourceKind('host').fallback, 'host');
  assert.equal(resourceKind('container').logo, 'docker');
});

test('hosts and images show their distribution, packages their ecosystem', async () => {
  const { osLogo, ecosystemLogo } = await import('../web/src/pages/security/cloud/resourceKinds.ts');
  const os = {
    'debian 12': 'debian',
    'ubuntu 22.04': 'ubuntu',
    'alpine 3.19': 'alpine',
    'red hat 9': 'redhat',
    'centos 7': 'centos',
    'amazon linux 2023': 'aws',
    'rocky linux 9': 'rockylinux',
    almalinux: 'almalinux',
    wolfi: 'chainguard',
    'photon os 5.0': 'linux',
  };
  for (const [s, logo] of Object.entries(os)) assert.equal(osLogo(s), logo, s);
  assert.equal(osLogo(''), null);
  const eco = {
    npm: 'npm',
    Maven: 'maven',
    PyPI: 'pypi',
    RubyGems: 'rubygems',
    'crates.io': 'rust',
    NuGet: 'nuget',
    Packagist: 'composer',
    Go: 'go',
    'Debian:12': 'debian',
    'Alpine:v3.19': 'alpine',
    'Ubuntu:22.04:LTS': 'ubuntu',
    'Amazon Linux:2023': 'aws',
  };
  for (const [e, logo] of Object.entries(eco)) assert.equal(ecosystemLogo(e), logo, e);
  assert.equal(ecosystemLogo('Hex'), null);
});

test('the resource types of the provenance lists read as names, not raw ids', () => {
  const names = {
    AwsS3Bucket: 'S3 Bucket',
    AwsRdsDbInstance: 'RDS DB Instance',
    AwsSqsQueue: 'SQS Queue',
    AwsEc2Instance: 'EC2 Instance',
    AwsLambdaFunction: 'Lambda Function',
    AwsIamRole: 'IAM Role',
    'Microsoft.Storage/storageAccounts': 'Storage Account',
    'Microsoft.KeyVault/vaults': 'Key Vault',
    'storage.googleapis.com/Bucket': 'Cloud Storage Bucket',
    kube_deployment: 'Deployment',
    kube_service: 'Service',
    container_image: 'Container Image',
    service: 'APM Service',
  };
  for (const [type, name] of Object.entries(names)) assert.equal(resourceKind(type).name, name, type);
  assert.equal(resourceKind('AwsS3Bucket').logo, 'tile:aws-s3');
  assert.equal(resourceKind('kube_deployment').kube, 'deployment');
  assert.equal(resourceKind('container_image').logo, 'docker');
  assert.equal(resourceKind('service').fallback, 'service');
});
