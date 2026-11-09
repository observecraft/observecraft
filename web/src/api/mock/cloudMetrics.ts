// Mock metrics of the AWS, Azure and Google Cloud services the cloud overview dashboards chart, with
// the tags our crawlers put on them (account and region, the resource's dimension tags). Fictional:
// the e-commerce demo's names, account 123456789012.
type Tags = Record<string, string>;

export interface CloudMetricDef {
  set: string;
  description: string;
  unit?: string;
  base: (e: Tags, r: () => number) => number;
  noise: number;
  integer?: boolean;
  max?: number;
}

const ACCOUNT = { aws_account: '123456789012' };
const APPS = ['storefront-api', 'order-service', 'catalog-service', 'inventory-service', 'billing', 'api-gateway'];
const AWS_REGIONS = ['eu-west-1', 'us-east-1'];
const AZ = { subscription_id: '9f1c2a7e-4b3d-4e8a-9c21-0d5e6f7a8b90' };
const GCP = { project_id: 'example-storefront' };

const each = <T>(n: number, f: (i: number) => T) => Array.from({ length: n }, (_, i) => f(i));

export const CLOUD_SETS: Record<string, Tags[]> = {
  aws_ec2: each(12, (i) => ({ ...ACCOUNT, region: AWS_REGIONS[i % 2], instanceid: `i-0a1b2c3d4e5f6${String(i).padStart(4, '0')}`, host: `i-0a1b2c3d4e5f6${String(i).padStart(4, '0')}` })),
  aws_rds: APPS.slice(0, 4).map((app, i) => ({ ...ACCOUNT, region: AWS_REGIONS[i % 2], dbinstanceidentifier: `${app}-db` })),
  aws_elb: ['storefront-classic', 'legacy-checkout'].map((n, i) => ({ ...ACCOUNT, region: AWS_REGIONS[i % 2], loadbalancername: n })),
  aws_alb: ['app/storefront-alb/50dc6c495c0c9188', 'app/api-gateway-alb/8a2b1c9d7e6f5a40', 'app/billing-alb/1f2e3d4c5b6a7980'].map((n, i) => ({
    ...ACCOUNT,
    region: AWS_REGIONS[i % 2],
    loadbalancer: n,
    targetgroup: `targetgroup/${n.split('/')[1].replace(/-alb$/, '')}/${n.split('/')[2]}`,
  })),
  aws_lambda: ['order-confirmation-email', 'image-resize', 'inventory-sync', 'payment-webhook', 'catalog-indexer'].map((n, i) => ({ ...ACCOUNT, region: AWS_REGIONS[i % 2], functionname: n })),
  aws_s3: ['example-storefront-assets', 'example-order-exports', 'example-logs-archive', 'example-catalog-images'].map((n) => ({
    ...ACCOUNT,
    region: 'eu-west-1',
    bucketname: n,
    storagetype: 'StandardStorage',
    filterid: 'EntireBucket',
  })),
  aws_ebs: each(14, (i) => ({ ...ACCOUNT, region: AWS_REGIONS[i % 2], volumeid: `vol-0a1b2c3d4e5f6${String(i).padStart(4, '0')}` })),
  azure_vm: APPS.map((app, i) => ({ ...AZ, resource_group: 'storefront', region: i % 2 ? 'northeurope' : 'westeurope', name: `${app}-vm` })),
  azure_sql: ['orders', 'catalog', 'billing'].map((n) => ({ ...AZ, resource_group: 'storefront', region: 'westeurope', name: n })),
  azure_app: ['storefront-web', 'api-gateway', 'order-functions'].map((n) => ({ ...AZ, resource_group: 'storefront', region: 'westeurope', name: n })),
  azure_lb: ['storefront-lb', 'internal-lb'].map((n) => ({ ...AZ, resource_group: 'network', region: 'westeurope', name: n })),
  azure_storage: ['storefrontassets', 'orderexports'].map((n) => ({ ...AZ, resource_group: 'storefront', region: 'westeurope', name: n })),
  gcp_gce: each(8, (i) => ({ ...GCP, zone: i % 2 ? 'europe-west1-c' : 'europe-west1-b', instance_id: `71234567890123${String(i).padStart(4, '0')}` })),
  gcp_cloudsql: ['example-storefront:orders', 'example-storefront:catalog'].map((n) => ({ ...GCP, region: 'europe-west1', database_id: n })),
  gcp_gke: APPS.slice(0, 5).map((app, i) => ({ ...GCP, cluster_name: i < 3 ? 'storefront-prod' : 'storefront-staging', namespace_name: 'shop', container_name: app })),
  gcp_run: ['storefront-web', 'checkout-api', 'recommendations'].flatMap((s) => ['2xx', '4xx', '5xx'].map((c) => ({ ...GCP, service_name: s, response_code_class: c }))),
  gcp_functions: ['order-created', 'resize-image', 'nightly-export'].flatMap((f) => ['ok', 'error'].map((status) => ({ ...GCP, function_name: f, status }))),
  gcp_lb: ['storefront-url-map', 'api-url-map'].map((n) => ({ ...GCP, url_map_name: n })),
};

const range = (lo: number, hi: number) => (_e: Tags, r: () => number) => lo + r() * (hi - lo);
const sometimes = (p: number, lo: number, hi: number) => (_e: Tags, r: () => number) => (r() < p ? lo + r() * (hi - lo) : 0);
const m = (set: string, description: string, unit: string | undefined, base: CloudMetricDef['base'], noise = 0.2, extra: Partial<CloudMetricDef> = {}): CloudMetricDef => ({
  set,
  description,
  unit,
  base,
  noise,
  ...extra,
});
const GIB = 2 ** 30;

export const CLOUD_METRICS: Record<string, CloudMetricDef> = {
  'aws.ec2.cpuutilization': m('aws_ec2', 'EC2 CPU utilization', 'percent', range(4, 70), 0.25, { max: 100 }),
  'aws.ec2.cpuutilization.maximum': m('aws_ec2', 'EC2 CPU utilization (maximum)', 'percent', range(20, 98), 0.2, { max: 100 }),
  'aws.ec2.status_check_failed.maximum': m('aws_ec2', 'EC2 failed status checks', undefined, sometimes(0.08, 1, 1), 0, { integer: true }),
  // The EC2 dashboard's status checks: a failed check now and then, on a few instances.
  'aws.ec2.status_check_failed': m('aws_ec2', 'EC2 failed status checks', undefined, (e, r) => (e.instanceid.endsWith('3') ? 1 + r() * 2 : 0), 0.5, { integer: true }),
  'aws.ec2.status_check_failed_instance': m('aws_ec2', 'EC2 failed instance checks', undefined, (e, r) => (e.instanceid.endsWith('3') ? 1 + r() * 2 : 0), 0.5, { integer: true }),
  'aws.ec2.status_check_failed_system': m('aws_ec2', 'EC2 failed system checks', undefined, (e, r) => (e.instanceid.endsWith('7') ? 1 + r() : 0), 0.5, { integer: true }),
  'aws.ec2.network_in.sum': m('aws_ec2', 'EC2 bytes received', 'byte', range(2e7, 4e8), 0.35),
  'aws.ec2.network_out.sum': m('aws_ec2', 'EC2 bytes sent', 'byte', range(1e7, 3e8), 0.35),
  'aws.rds.database_connections': m('aws_rds', 'RDS connections', undefined, range(8, 180), 0.2, { integer: true }),
  'aws.rds.cpuutilization': m('aws_rds', 'RDS CPU utilization', 'percent', range(6, 60), 0.2, { max: 100 }),
  'aws.rds.free_storage_space': m('aws_rds', 'RDS free storage', 'byte', range(20 * GIB, 400 * GIB), 0.01),
  'aws.rds.read_iops': m('aws_rds', 'RDS read IOPS', undefined, range(40, 900), 0.3),
  'aws.rds.read_latency': m('aws_rds', 'RDS read latency', 'second', range(0.0005, 0.008), 0.3),
  'aws.rds.write_latency': m('aws_rds', 'RDS write latency', 'second', range(0.001, 0.012), 0.3),
  'aws.elb.healthy_host_count': m('aws_elb', 'ELB healthy hosts', undefined, range(2, 6), 0, { integer: true }),
  'aws.elb.un_healthy_host_count': m('aws_elb', 'ELB unhealthy hosts', undefined, sometimes(0.3, 1, 2), 0, { integer: true }),
  'aws.elb.request_count.sum': m('aws_elb', 'ELB requests', undefined, range(2_000, 20_000), 0.3),
  'aws.elb.httpcode_elb_5xx.sum': m('aws_elb', 'ELB 5xx responses', undefined, sometimes(0.5, 1, 30), 0.6),
  'aws.elb.latency': m('aws_elb', 'ELB latency', 'second', range(0.02, 0.3), 0.25),
  'aws.applicationelb.healthy_host_count': m('aws_alb', 'ALB healthy targets', undefined, range(2, 9), 0, { integer: true }),
  'aws.applicationelb.un_healthy_host_count': m('aws_alb', 'ALB unhealthy targets', undefined, sometimes(0.25, 1, 1), 0, { integer: true }),
  'aws.applicationelb.request_count.sum': m('aws_alb', 'ALB requests', undefined, range(10_000, 90_000), 0.3),
  'aws.applicationelb.httpcode_elb_5xx_count.sum': m('aws_alb', 'ALB 5xx responses', undefined, sometimes(0.6, 1, 40), 0.6),
  'aws.applicationelb.target_response_time': m('aws_alb', 'ALB target response time', 'second', range(0.03, 0.45), 0.25),
  // The Lambda dashboard's bare names (Datadog's statistic): errors and throttles a small share of the invocations.
  'aws.lambda.invocations': m('aws_lambda', 'Lambda invocations', 'invocation', range(200, 9_000), 0.35),
  'aws.lambda.errors': m('aws_lambda', 'Lambda errors', 'error', range(1, 40), 0.6),
  'aws.lambda.throttles': m('aws_lambda', 'Lambda throttles', 'throttle', range(0.5, 12), 0.7),
  'aws.lambda.invocations.sum': m('aws_lambda', 'Lambda invocations', undefined, range(200, 9_000), 0.35),
  'aws.lambda.errors.sum': m('aws_lambda', 'Lambda errors', undefined, sometimes(0.6, 1, 60), 0.6),
  'aws.lambda.duration': m('aws_lambda', 'Lambda duration', 'millisecond', range(30, 1_800), 0.25),
  'aws.lambda.throttles.sum': m('aws_lambda', 'Lambda throttles', undefined, sometimes(0.3, 1, 15), 0.7),
  'aws.lambda.concurrent_executions.maximum': m('aws_lambda', 'Lambda concurrent executions', undefined, range(2, 60), 0.3, { integer: true }),
  'aws.s3.bucket_size_bytes': m('aws_s3', 'S3 bucket size', 'byte', range(5 * GIB, 900 * GIB), 0.002),
  'aws.s3.number_of_objects': m('aws_s3', 'S3 objects', undefined, range(10_000, 4_000_000), 0.002, { integer: true }),
  // The S3 dashboard's bare names: errors a small share of the requests.
  'aws.s3.all_requests': m('aws_s3', 'S3 requests', 'request', range(500, 30_000), 0.35),
  'aws.s3.4xx_errors': m('aws_s3', 'S3 4xx errors', undefined, range(1, 80), 0.6),
  'aws.s3.5xx_errors': m('aws_s3', 'S3 5xx errors', undefined, range(0.2, 6), 0.7),
  'aws.s3.all_requests.sum': m('aws_s3', 'S3 requests', undefined, range(500, 30_000), 0.35),
  'aws.s3.4xx_errors.sum': m('aws_s3', 'S3 4xx errors', undefined, sometimes(0.5, 1, 80), 0.6),
  'aws.ebs.volume_queue_length': m('aws_ebs', 'EBS queue length', undefined, range(0.01, 2.5), 0.4),
  'aws.ebs.burst_balance': m('aws_ebs', 'EBS burst balance', 'percent', range(60, 100), 0.05, { max: 100 }),
  'aws.ebs.volume_idle_time.sum': m('aws_ebs', 'EBS idle time', 'second', range(10, 290), 0.2),
  'aws.ebs.volume_read_ops.sum': m('aws_ebs', 'EBS read operations', undefined, range(100, 30_000), 0.35),
  'aws.ebs.volume_write_ops.sum': m('aws_ebs', 'EBS write operations', undefined, range(500, 60_000), 0.35),
  'aws.ebs.volume_read_bytes.sum': m('aws_ebs', 'EBS bytes read', 'byte', range(1e7, 9e8), 0.35),
  'aws.ebs.volume_write_bytes.sum': m('aws_ebs', 'EBS bytes written', 'byte', range(5e7, 2e9), 0.35),
  'azure.vm.percentage_cpu': m('azure_vm', 'VM CPU', 'percent', range(5, 75), 0.25, { max: 100 }),
  'azure.vm.percentage_cpu.maximum': m('azure_vm', 'VM CPU (maximum)', 'percent', range(30, 99), 0.2, { max: 100 }),
  'azure.vm.network_in_total.total': m('azure_vm', 'VM bytes received', 'byte', range(1e7, 3e8), 0.35),
  'azure.vm.network_out_total.total': m('azure_vm', 'VM bytes sent', 'byte', range(1e7, 2e8), 0.35),
  'azure.vm.available_memory_bytes': m('azure_vm', 'VM available memory', 'byte', range(GIB, 12 * GIB), 0.08),
  'azure.vm.disk_read_bytes.total': m('azure_vm', 'VM disk bytes read', 'byte', range(1e6, 2e8), 0.4),
  'azure.vm.disk_write_bytes.total': m('azure_vm', 'VM disk bytes written', 'byte', range(5e6, 4e8), 0.4),
  'azure.sql_servers_databases.connection_failed.total': m('azure_sql', 'Failed connections', undefined, sometimes(0.4, 1, 6), 0.6, { integer: true }),
  'azure.sql_servers_databases.connection_successful.total': m('azure_sql', 'Successful connections', undefined, range(50, 800), 0.3, { integer: true }),
  'azure.sql_servers_databases.cpu_percent': m('azure_sql', 'Database CPU', 'percent', range(5, 70), 0.25, { max: 100 }),
  'azure.sql_servers_databases.deadlock.total': m('azure_sql', 'Deadlocks', undefined, sometimes(0.2, 1, 2), 0.5, { integer: true }),
  'azure.sql_servers_databases.storage_percent': m('azure_sql', 'Storage used', 'percent', range(15, 85), 0.01, { max: 100 }),
  'azure.sql_servers_databases.dtu_consumption_percent': m('azure_sql', 'DTU consumption', 'percent', range(10, 90), 0.25, { max: 100 }),
  'azure.app_services.requests.total': m('azure_app', 'Requests', undefined, range(2_000, 40_000), 0.3),
  'azure.app_services.http5xx.total': m('azure_app', '5xx responses', undefined, sometimes(0.6, 1, 50), 0.6),
  'azure.app_services.function_execution_count.total': m('azure_app', 'Function executions', undefined, (e, r) => (e.name.includes('functions') ? 500 + r() * 5_000 : 0), 0.3),
  'azure.app_services.http_response_time': m('azure_app', 'Response time', 'second', range(0.05, 0.9), 0.25),
  'azure.network_loadbalancers.vip_availability': m('azure_lb', 'Data path availability', 'percent', range(99.5, 100), 0.002, { max: 100 }),
  'azure.network_loadbalancers.dip_availability': m('azure_lb', 'Health probe status', 'percent', range(92, 100), 0.02, { max: 100 }),
  'azure.network_loadbalancers.byte_count.total': m('azure_lb', 'Bytes', 'byte', range(1e8, 2e9), 0.3),
  'azure.storage.availability': m('azure_storage', 'Availability', 'percent', range(99.9, 100), 0.0005, { max: 100 }),
  'azure.storage.used_capacity': m('azure_storage', 'Used capacity', 'byte', range(50 * GIB, 2_000 * GIB), 0.002),
  'azure.storage.transactions.total': m('azure_storage', 'Transactions', undefined, range(1_000, 80_000), 0.3),
  'azure.storage.success_server_latency': m('azure_storage', 'Server latency', 'millisecond', range(2, 25), 0.3),
  'gcp.gce.instance.cpu.utilization': m('gcp_gce', 'Instance CPU utilization', 'fraction', range(0.05, 0.75), 0.25, { max: 1 }),
  'gcp.gce.instance.network.received_bytes_count': m('gcp_gce', 'Bytes received', 'byte', range(1e7, 3e8), 0.35),
  'gcp.gce.instance.network.sent_bytes_count': m('gcp_gce', 'Bytes sent', 'byte', range(1e7, 2e8), 0.35),
  'gcp.gce.instance.disk.read_bytes_count': m('gcp_gce', 'Disk bytes read', 'byte', range(1e6, 1e8), 0.4),
  'gcp.cloudsql.database.network.connections': m('gcp_cloudsql', 'Connections', undefined, range(10, 150), 0.2, { integer: true }),
  'gcp.cloudsql.database.cpu.utilization': m('gcp_cloudsql', 'CPU utilization', 'fraction', range(0.05, 0.6), 0.25, { max: 1 }),
  'gcp.cloudsql.database.memory.utilization': m('gcp_cloudsql', 'Memory utilization', 'fraction', range(0.3, 0.8), 0.05, { max: 1 }),
  'gcp.cloudsql.database.disk.bytes_used': m('gcp_cloudsql', 'Disk used', 'byte', range(10 * GIB, 200 * GIB), 0.002),
  'gcp.cloudsql.database.disk.utilization': m('gcp_cloudsql', 'Disk utilization', 'fraction', range(0.2, 0.85), 0.002, { max: 1 }),
  'gcp.gke.node.cpu.allocatable_utilization': m('gcp_gke', 'Node CPU allocatable utilization', 'fraction', range(0.2, 0.8), 0.15, { max: 1 }),
  'gcp.gke.node.memory.allocatable_utilization': m('gcp_gke', 'Node memory allocatable utilization', 'fraction', range(0.3, 0.85), 0.05, { max: 1 }),
  'gcp.gke.container.memory.used_bytes': m('gcp_gke', 'Container memory', 'byte', range(100 * 2 ** 20, 2 * GIB), 0.08),
  'gcp.gke.container.restart_count': m('gcp_gke', 'Container restarts', undefined, sometimes(0.3, 1, 4), 0.5, { integer: true }),
  'gcp.run.request_count': m('gcp_run', 'Requests', undefined, (e, r) => (e.response_code_class === '2xx' ? 1_000 + r() * 9_000 : e.response_code_class === '4xx' ? r() * 300 : r() * 40), 0.3),
  'gcp.cloudfunctions.function.execution_count': m('gcp_functions', 'Executions', undefined, (e, r) => (e.status === 'ok' ? 200 + r() * 4_000 : r() * 60), 0.35),
  'gcp.loadbalancing.https.request_count': m('gcp_lb', 'Requests', undefined, range(5_000, 60_000), 0.3),
  'gcp.loadbalancing.https.request_bytes_count': m('gcp_lb', 'Request bytes', 'byte', range(1e7, 2e8), 0.3),
  'gcp.loadbalancing.https.response_bytes_count': m('gcp_lb', 'Response bytes', 'byte', range(1e8, 3e9), 0.3),
};
