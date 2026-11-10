// Dashboards designed per AWS service, as Datadog has one for each service of its AWS integration: each follows its
// service's model (what a queue, a load balancer or a function is judged by) instead of one template. The structure
// (an overview with the service's logo, key numbers and monitors, then a group per question, its logs last) follows
// the kind of dashboards Datadog ships; titles, notes and layouts are ours, and only metric names our AWS crawler
// publishes are charted (aws/naming.rs: a metric's bare name carries Datadog's statistic, `.sum` / `.maximum` /
// `.minimum` the others, `.p95` its percentiles). scripts/cloud-service-dashboards.mjs turns them into dashboards;
// the services left out get its generated layout.
import { metric, note, queryValue, table, timeseries, treemap } from './cloud-dashboards.mjs';

// ---------------------------------------------------------------- widgets (Datadog's format) with their size

/** A widget and the room it takes in its group (12 columns). `span` gives it its own time frame (daily metrics). */
const sized = (definition, width, height, span) => ({ definition: span ? { ...definition, time: { live_span: span } } : definition, width, height });
/** A query's time aggregation from its space aggregation: totals of sums, peaks of maxima, averages of the rest. */
const timeAgg = (query) => ({ sum: 'sum', max: 'max', min: 'min' })[query.split(':')[0]] ?? 'avg';
const metrics = (qs, agg) => [qs].flat().map((q, i) => metric(q, agg ?? timeAgg(q), `query${i + 1}`));
const BAD = [{ comparator: '>', value: 0, palette: 'red_on_white' }];
const OK = [{ comparator: '>', value: 0, palette: 'green_on_white' }];

/** A key number: one query, or several combined by `formula`; `bad` turns it red above zero, `good` green. */
export const qv = (title, qs, { agg, formula = 'query1', precision = 0, unit, bad, good, span } = {}) =>
  sized(queryValue(title, metrics(qs, agg), { formula, precision, unit, formats: bad ? BAD : good ? OK : [] }), 2, 2, span);

/** A timeseries of one or more queries (named by `alias`), or of `formulas` ([formula, alias]) over them. */
export function ts(title, qs, { alias = [], formulas, display = 'line', palette = 'dog_classic', w = 6, h = 2, span } = {}) {
  const queries = metrics(qs);
  const f = formulas ? formulas.map(([formula, a]) => ({ formula, ...(a ? { alias: a } : {}) })) : queries.map((q, i) => ({ formula: q.name, ...(alias[i] ? { alias: alias[i] } : {}) }));
  return sized(timeseries(title, queries, { formulas: f, display, palette }), w, h, span);
}

/** The top (or bottom, `order: 'asc'`) groups of a query, or of a formula over several. */
export function top(title, qs, { agg, formula = 'query1', order = 'desc', count = 10, w = 6, h = 2, span, palette } = {}) {
  return sized(
    {
      type: 'toplist',
      title,
      title_size: '16',
      title_align: 'left',
      requests: [
        {
          response_format: 'scalar',
          queries: metrics(qs, agg),
          formulas: [{ formula }],
          sort: { count, order_by: [{ type: 'formula', index: 0, order }] },
          ...(palette ? { style: { palette } } : {}),
        },
      ],
    },
    w,
    h,
    span,
  );
}

/** A table of a resource's numbers: columns [alias, query, time aggregation]. */
export const tbl = (title, columns, { w = 12, h = 3 } = {}) => sized(table(title, columns), w, h);
/** How a metric spreads across the resources over time. */
export const heat = (title, q, { w = 6, h = 2 } = {}) =>
  sized(
    {
      type: 'heatmap',
      title,
      title_size: '16',
      title_align: 'left',
      requests: [{ response_format: 'timeseries', queries: [metric(q)], formulas: [{ formula: 'query1' }], style: { palette: 'dog_classic' } }],
    },
    w,
    h,
  );
/** Each resource's share of a total. */
export const tree = (title, q, { agg, w = 6, h = 2, span } = {}) => sized(treemap(title, q, agg ?? timeAgg(q)), w, h, span);
/** Each resource's value against the week before. */
export const chg = (title, q, { increaseGood = true, w = 6, h = 2, span = '1w' } = {}) =>
  sized(
    {
      type: 'change',
      title,
      title_size: '16',
      title_align: 'left',
      requests: [{ q, change_type: 'relative', compare_to: 'week_before', order_by: 'change', order_dir: 'desc', increase_good: increaseGood }],
    },
    w,
    h,
    span,
  );
/** A short explanation inside a group. */
export const txt = (content, { w = 12, h = 2 } = {}) => sized(note(content, 'gray', { align: 'left' }), w, h);
/** A themed group: its widgets flow in its own columns. */
export const group = (title, widgets, { color = 'orange', w = 12 } = {}) => ({ group: true, title, widgets, color, width: w });

/** Queries over a dashboard's filter (`$account,$region,…`): `sum(metric, by, ...filters)`. */
export function queriesOver(f) {
  const make =
    (space) =>
    (name, by, ...filters) =>
      `${space}:${name}{${[f, ...filters].filter(Boolean).join(',')}}${by ? ` by {${by}}` : ''}`;
  return { sum: make('sum'), avg: make('avg'), max: make('max'), min: make('min') };
}

// ---------------------------------------------------------------- the services

/**
 * namespace → { vars: the template variables after account and region (the tags the crawler gives its dimensions),
 * monitor: the `integration:` tag of its monitors, note: what it shows (our words), logs: [[title, query]],
 * units: CloudWatch metric → unit (on every name the crawler gives it), build({ sum, avg, max, min, f }) → { kpis, groups } }.
 */
export const AWS_LAYOUTS = {
  'AWS/EC2': {
    vars: ['instanceid'],
    monitor: 'amazon_ec2',
    note: 'Instances seen from the hypervisor: status checks, CPU and burst credits, network and the EBS volumes attached to them. Memory and disk usage inside the instance come from the Agent, not CloudWatch.',
    units: {
      CPUUtilization: 'percent',
      NetworkIn: 'byte',
      NetworkOut: 'byte',
      NetworkPacketsIn: 'packet',
      NetworkPacketsOut: 'packet',
      DiskReadBytes: 'byte',
      DiskWriteBytes: 'byte',
      DiskReadOps: 'operation',
      DiskWriteOps: 'operation',
      EBSReadBytes: 'byte',
      EBSWriteBytes: 'byte',
      EBSReadOps: 'operation',
      EBSWriteOps: 'operation',
      'EBSIOBalance%': 'percent',
      'EBSByteBalance%': 'percent',
    },
    build: ({ sum, avg, max, min }) => ({
      kpis: [
        qv('Instances reporting', avg('aws.ec2.cpuutilization', 'instanceid'), { formula: 'count_not_null(query1)', good: true }),
        qv('Avg CPU', avg('aws.ec2.cpuutilization'), { precision: 1 }),
        qv('Peak CPU', max('aws.ec2.cpuutilization.maximum'), { precision: 1 }),
        qv('Failed status checks', sum('aws.ec2.status_check_failed'), { bad: true }),
        qv('Network in', sum('aws.ec2.network_in.sum')),
        qv('Network out', sum('aws.ec2.network_out.sum')),
      ],
      groups: [
        group('Status checks', [
          ts('Instance check failures', sum('aws.ec2.status_check_failed_instance', 'instanceid'), { display: 'bars', palette: 'warm' }),
          ts('System check failures', sum('aws.ec2.status_check_failed_system', 'instanceid'), { display: 'bars', palette: 'warm' }),
          top('Instances failing checks', sum('aws.ec2.status_check_failed', 'instanceid'), { palette: 'warm' }),
          ts('Metadata calls without a token (IMDSv1)', sum('aws.ec2.metadata_no_token', 'instanceid'), { display: 'bars' }),
        ]),
        group('CPU', [
          top('Busiest instances', avg('aws.ec2.cpuutilization', 'instanceid'), { w: 4, h: 4, palette: 'orange' }),
          ts('CPU utilization by instance', avg('aws.ec2.cpuutilization', 'instanceid'), { w: 8 }),
          heat('CPU spread across instances', avg('aws.ec2.cpuutilization', 'instanceid'), { w: 8 }),
          ts('CPU credit balance (burstable)', avg('aws.ec2.cpucredit_balance', 'instanceid')),
          ts('Surplus credits charged', sum('aws.ec2.cpusurplus_credits_charged.sum', 'instanceid'), { display: 'bars', palette: 'warm' }),
        ]),
        group('Network', [
          ts('Traffic in and out', [sum('aws.ec2.network_in.sum'), sum('aws.ec2.network_out.sum')], {
            formulas: [
              ['query1', 'in'],
              ['0 - query2', 'out'],
            ],
            display: 'bars',
          }),
          ts('Packets in and out', [sum('aws.ec2.network_packets_in.sum'), sum('aws.ec2.network_packets_out.sum')], { alias: ['in', 'out'] }),
          top('Most traffic sent', sum('aws.ec2.network_out.sum', 'instanceid')),
          top('Most traffic received', sum('aws.ec2.network_in.sum', 'instanceid')),
        ]),
        group('Disks', [
          ts('EBS throughput', [sum('aws.ec2.ebsread_bytes.sum'), sum('aws.ec2.ebswrite_bytes.sum')], { alias: ['read', 'written'] }),
          ts('EBS operations', [sum('aws.ec2.ebsread_ops.sum'), sum('aws.ec2.ebswrite_ops.sum')], { alias: ['reads', 'writes'] }),
          top('Lowest EBS I/O balance', min('aws.ec2.ebsiobalance', 'instanceid'), { order: 'asc', w: 4 }),
          top('Lowest EBS byte balance', min('aws.ec2.ebsbyte_balance', 'instanceid'), { order: 'asc', w: 4 }),
          ts('Instance store throughput', [sum('aws.ec2.disk_read_bytes.sum'), sum('aws.ec2.disk_write_bytes.sum')], { alias: ['read', 'written'], w: 4 }),
        ]),
      ],
    }),
  },

  'AWS/EBS': {
    vars: ['volumeid'],
    monitor: 'amazon_ebs',
    note: 'Block volumes: operations and throughput, the time each operation takes, queue length and the burst balance of gp2, st1 and sc1 volumes.',
    units: {
      VolumeReadBytes: 'byte',
      VolumeWriteBytes: 'byte',
      VolumeReadOps: 'operation',
      VolumeWriteOps: 'operation',
      VolumeTotalReadTime: 'second',
      VolumeTotalWriteTime: 'second',
      VolumeIdleTime: 'second',
      BurstBalance: 'percent',
    },
    build: ({ sum, avg, max, min }) => ({
      kpis: [
        qv('Volumes reporting', avg('aws.ebs.volume_idle_time', 'volumeid'), { formula: 'count_not_null(query1)', good: true }),
        qv('Read operations', sum('aws.ebs.volume_read_ops')),
        qv('Write operations', sum('aws.ebs.volume_write_ops')),
        qv('Longest queue', max('aws.ebs.volume_queue_length'), { precision: 2 }),
        qv('Lowest burst balance', min('aws.ebs.burst_balance'), { precision: 1 }),
      ],
      groups: [
        group('Throughput', [
          ts('Read operations by volume', sum('aws.ebs.volume_read_ops', 'volumeid'), { display: 'bars' }),
          ts('Write operations by volume', sum('aws.ebs.volume_write_ops', 'volumeid'), { display: 'bars' }),
          ts('Bytes read by volume', sum('aws.ebs.volume_read_bytes.sum', 'volumeid')),
          ts('Bytes written by volume', sum('aws.ebs.volume_write_bytes.sum', 'volumeid')),
        ]),
        group('Latency', [
          ts('Read time per operation', [sum('aws.ebs.volume_total_read_time.sum', 'volumeid'), sum('aws.ebs.volume_read_ops', 'volumeid')], { formulas: [['query1 / query2', 'read time']] }),
          ts('Write time per operation', [sum('aws.ebs.volume_total_write_time.sum', 'volumeid'), sum('aws.ebs.volume_write_ops', 'volumeid')], { formulas: [['query1 / query2', 'write time']] }),
        ]),
        group('Queue and burst credits', [
          heat('Queue length across volumes', avg('aws.ebs.volume_queue_length', 'volumeid')),
          top('Longest queues', avg('aws.ebs.volume_queue_length', 'volumeid'), { palette: 'orange' }),
          top('Lowest burst balance', min('aws.ebs.burst_balance', 'volumeid'), { order: 'asc' }),
          top('Most idle volumes', avg('aws.ebs.volume_idle_time', 'volumeid')),
        ]),
      ],
    }),
  },

  'AWS/S3': {
    vars: ['bucketname', 'filterid'],
    monitor: 'amazon_s3',
    note: 'Buckets: their size and objects (daily storage metrics, published once a day for the day before), and with request metrics enabled on a bucket its requests, errors, latency and transfer.',
    logs: [['S3 access logs', 'source:s3']],
    units: {
      BucketSizeBytes: 'byte',
      NumberOfObjects: 'object',
      AllRequests: 'request',
      GetRequests: 'request',
      PutRequests: 'request',
      DeleteRequests: 'request',
      HeadRequests: 'request',
      PostRequests: 'request',
      ListRequests: 'request',
      BytesDownloaded: 'byte',
      BytesUploaded: 'byte',
      FirstByteLatency: 'millisecond',
      TotalRequestLatency: 'millisecond',
    },
    build: ({ sum, avg }) => ({
      kpis: [
        qv('Storage', sum('aws.s3.bucket_size_bytes'), { agg: 'last', span: '1w' }),
        qv('Objects', sum('aws.s3.number_of_objects'), { agg: 'last', span: '1w' }),
        qv('Requests', sum('aws.s3.all_requests')),
        qv('4xx errors', sum('aws.s3.4xx_errors'), { bad: true }),
        qv('5xx errors', sum('aws.s3.5xx_errors'), { bad: true }),
        qv('First byte p95', avg('aws.s3.first_byte_latency.p95')),
      ],
      groups: [
        group('Storage (daily)', [
          ts('Bucket size', sum('aws.s3.bucket_size_bytes', 'bucketname'), { display: 'bars', span: '1mo' }),
          top('Largest buckets', sum('aws.s3.bucket_size_bytes', 'bucketname'), { agg: 'last', span: '1w' }),
          tree('Objects by bucket', sum('aws.s3.number_of_objects', 'bucketname'), { agg: 'last', span: '1w' }),
          chg('Size against the week before', sum('aws.s3.bucket_size_bytes', 'bucketname')),
          ts('Storage by class', sum('aws.s3.bucket_size_bytes', 'storagetype'), { display: 'area', span: '1mo', w: 12 }),
        ]),
        group('Requests', [
          ts('Requests by bucket', sum('aws.s3.all_requests', 'bucketname'), { display: 'bars' }),
          ts(
            'Requests by type',
            ['get', 'put', 'delete', 'head', 'post', 'list'].map((t) => sum(`aws.s3.${t}_requests`)),
            { alias: ['GET', 'PUT', 'DELETE', 'HEAD', 'POST', 'LIST'] },
          ),
          top('Busiest buckets', sum('aws.s3.all_requests', 'bucketname'), { w: 12 }),
        ]),
        group('Errors', [
          ts('4xx errors by bucket', sum('aws.s3.4xx_errors', 'bucketname'), { display: 'bars', palette: 'warm', w: 4 }),
          ts('5xx errors by bucket', sum('aws.s3.5xx_errors', 'bucketname'), { display: 'bars', palette: 'warm', w: 4 }),
          top('Highest error rate', [sum('aws.s3.4xx_errors', 'bucketname'), sum('aws.s3.5xx_errors', 'bucketname'), sum('aws.s3.all_requests', 'bucketname')], {
            formula: '100 * (query1 + query2) / query3',
            w: 4,
            palette: 'warm',
          }),
        ]),
        group('Latency', [
          ts('Time to first byte', [avg('aws.s3.first_byte_latency.p50'), avg('aws.s3.first_byte_latency.p95'), avg('aws.s3.first_byte_latency.p99')], { alias: ['p50', 'p95', 'p99'] }),
          ts('Total request latency p99 by bucket', avg('aws.s3.total_request_latency.p99', 'bucketname')),
          top('Slowest buckets (first byte p95)', avg('aws.s3.first_byte_latency.p95', 'bucketname'), { w: 12, palette: 'orange' }),
        ]),
        group('Data transfer', [
          ts('Downloaded and uploaded', [sum('aws.s3.bytes_downloaded'), sum('aws.s3.bytes_uploaded')], {
            formulas: [
              ['query1', 'downloaded'],
              ['0 - query2', 'uploaded'],
            ],
            display: 'bars',
            w: 8,
          }),
          top('Most downloaded', sum('aws.s3.bytes_downloaded', 'bucketname'), { w: 4 }),
        ]),
      ],
    }),
  },

  'AWS/RDS': {
    vars: ['dbinstanceidentifier', 'dbclusteridentifier'],
    monitor: 'amazon_rds',
    note: 'Database instances and Aurora clusters: resources and load, connections, query latency (Aurora adds per-statement latency and throughput), storage and I/O. Database internals come from the Agent’s database checks.',
    logs: [['RDS logs', 'source:rds']],
    build: ({ sum, avg, max }) => {
      const i = 'dbinstanceidentifier:*';
      const c = 'dbclusteridentifier:*';
      const by = 'dbinstanceidentifier';
      return {
        kpis: [
          qv('Connections', sum('aws.rds.database_connections', null, i)),
          qv('Avg CPU', avg('aws.rds.cpuutilization', null, i), { precision: 1 }),
          qv('DB load (sessions)', avg('aws.rds.dbload', null, i), { precision: 1 }),
          qv('Free storage', sum('aws.rds.free_storage_space', null, i), { agg: 'last' }),
          qv('Highest read latency', max('aws.rds.read_latency', null, i), { precision: 4 }),
          qv('Highest replica lag', max('aws.rds.replica_lag', null, i), { precision: 1 }),
        ],
        groups: [
          group('Resources', [
            top('Highest CPU', avg('aws.rds.cpuutilization', by, i), { w: 4, h: 4, palette: 'orange' }),
            ts('CPU by instance', avg('aws.rds.cpuutilization', by, i), { w: 8 }),
            ts('Freeable memory by instance', avg('aws.rds.freeable_memory', by, i), { w: 4 }),
            ts('Swap used by instance', avg('aws.rds.swap_usage', by, i), { w: 4 }),
            top('Least freeable memory', avg('aws.rds.freeable_memory', by, i), { order: 'asc', w: 4 }),
            tree('Disk queue depth', avg('aws.rds.disk_queue_depth', by, i), { w: 4 }),
            ts('Burst balance (gp2)', avg('aws.rds.burst_balance', by, i), { w: 4 }),
          ]),
          group('Connections and load', [
            ts('Connections by instance', avg('aws.rds.database_connections', by, i)),
            ts('DB load: on CPU and waiting', [avg('aws.rds.dbload_cpu', null, i), avg('aws.rds.dbload_non_cpu', null, i)], { alias: ['CPU', 'waits'], display: 'area' }),
            ts('DB load per vCPU', avg('aws.rds.dbload_relative_to_num_vcpus', by, i)),
            ts('Aurora Serverless capacity (ACUs)', avg('aws.rds.serverless_database_capacity', by, i)),
          ]),
          group('Query performance', [
            ts('Read latency by instance', avg('aws.rds.read_latency', by, i)),
            ts('Write latency by instance', avg('aws.rds.write_latency', by, i)),
            ts('Aurora statement latency', [avg('aws.rds.select_latency', null, i), avg('aws.rds.dmllatency', null, i), avg('aws.rds.commit_latency', null, i)], {
              alias: ['select', 'DML', 'commit'],
            }),
            ts('Aurora statement throughput', [avg('aws.rds.select_throughput', null, i), avg('aws.rds.dmlthroughput', null, i), avg('aws.rds.commit_throughput', null, i)], {
              alias: ['select', 'DML', 'commit'],
            }),
            ts('Replica lag by instance', avg('aws.rds.replica_lag', by, i)),
            ts('Aurora replica lag (cluster maximum)', avg('aws.rds.aurora_replica_lag_maximum', 'dbclusteridentifier', c)),
            ts('Deadlocks', avg('aws.rds.deadlocks', by, i), { display: 'bars', palette: 'warm' }),
            ts('Buffer cache hit ratio', avg('aws.rds.buffer_cache_hit_ratio', by, i)),
          ]),
          group('Storage and I/O', [
            tbl('Instances', [
              ['Free storage', avg('aws.rds.free_storage_space', by, i)],
              ['Read IOPS', avg('aws.rds.read_iops', by, i)],
              ['Write IOPS', avg('aws.rds.write_iops', by, i)],
              ['Queue depth', avg('aws.rds.disk_queue_depth', by, i)],
            ]),
            ts('Read and write IOPS', [avg('aws.rds.read_iops', null, i), avg('aws.rds.write_iops', null, i)], { alias: ['read', 'write'] }),
            ts('Read and write throughput', [avg('aws.rds.read_throughput', null, i), avg('aws.rds.write_throughput', null, i)], { alias: ['read', 'write'] }),
            ts('Aurora volume used', avg('aws.rds.volume_bytes_used', 'dbclusteridentifier', c)),
            ts('Aurora volume IOPS', [avg('aws.rds.volume_read_iops', null, c), avg('aws.rds.volume_write_iops', null, c)], { alias: ['read', 'write'] }),
            ts('Log disk usage', [avg('aws.rds.transaction_logs_disk_usage', null, i), avg('aws.rds.bin_log_disk_usage', null, i)], { alias: ['transaction logs', 'binary logs'] }),
            ts('PostgreSQL transaction IDs used', max('aws.rds.maximum_used_transaction_ids', by, i)),
          ]),
          group('Network', [
            ts('Network throughput', [sum('aws.rds.network_receive_throughput', null, i), sum('aws.rds.network_transmit_throughput', null, i)], {
              formulas: [
                ['query1', 'received'],
                ['0 - query2', 'sent'],
              ],
            }),
            top('Least free storage', avg('aws.rds.free_storage_space', by, i), { order: 'asc' }),
          ]),
        ],
      };
    },
  },

  'AWS/ApplicationELB': {
    vars: ['loadbalancer', 'targetgroup'],
    monitor: 'amazon_elb',
    note: 'Application Load Balancers: the health of their targets, traffic and response time (p95 and p99), and HTTP responses by where they came from, the targets or the load balancer itself.',
    logs: [['Load balancer access logs', 'source:elb']],
    units: {
      TargetResponseTime: 'second',
      RequestCount: 'request',
      ActiveConnectionCount: 'connection',
      NewConnectionCount: 'connection',
      RejectedConnectionCount: 'connection',
      HTTPCode_Target_2XX_Count: 'response',
      HTTPCode_Target_3XX_Count: 'response',
      HTTPCode_Target_4XX_Count: 'response',
      HTTPCode_Target_5XX_Count: 'response',
      HTTPCode_ELB_3XX_Count: 'response',
      HTTPCode_ELB_4XX_Count: 'response',
      HTTPCode_ELB_5XX_Count: 'response',
      HTTPCode_ELB_502_Count: 'response',
      HTTPCode_ELB_503_Count: 'response',
      HTTPCode_ELB_504_Count: 'response',
      HealthyHostCount: 'host',
      UnHealthyHostCount: 'host',
    },
    build: ({ sum, avg, max }) => ({
      kpis: [
        qv('Healthy targets', sum('aws.applicationelb.healthy_host_count'), { agg: 'min', good: true }),
        qv('Unhealthy targets', sum('aws.applicationelb.un_healthy_host_count'), { agg: 'max', bad: true }),
        qv('Requests', sum('aws.applicationelb.request_count')),
        qv('Target 5xx', sum('aws.applicationelb.httpcode_target_5xx_count'), { bad: true }),
        qv('Load balancer 5xx', sum('aws.applicationelb.httpcode_elb_5xx_count'), { bad: true }),
        qv('Response time p95', avg('aws.applicationelb.target_response_time.p95'), { precision: 3 }),
      ],
      groups: [
        group('Target health', [
          ts('Healthy targets by load balancer', sum('aws.applicationelb.healthy_host_count', 'loadbalancer'), { palette: 'green' }),
          ts('Unhealthy targets by target group', sum('aws.applicationelb.un_healthy_host_count', 'targetgroup'), { display: 'bars', palette: 'warm' }),
          top('Load balancers with unhealthy targets', max('aws.applicationelb.un_healthy_host_count', 'loadbalancer'), { w: 12, palette: 'warm' }),
        ]),
        group('Traffic and performance', [
          ts('Requests by load balancer', sum('aws.applicationelb.request_count', 'loadbalancer'), { display: 'bars' }),
          ts('Target response time', [avg('aws.applicationelb.target_response_time.p50'), avg('aws.applicationelb.target_response_time.p95'), avg('aws.applicationelb.target_response_time.p99')], {
            alias: ['p50', 'p95', 'p99'],
          }),
          ts('Active connections', sum('aws.applicationelb.active_connection_count', 'loadbalancer'), { w: 4 }),
          ts('New connections', sum('aws.applicationelb.new_connection_count', 'loadbalancer'), { w: 4 }),
          ts('Rejected connections', sum('aws.applicationelb.rejected_connection_count', 'loadbalancer'), { display: 'bars', palette: 'warm', w: 4 }),
          top('Slowest load balancers (p95)', avg('aws.applicationelb.target_response_time.p95', 'loadbalancer'), { w: 12, palette: 'orange' }),
        ]),
        group('HTTP responses', [
          ts(
            'Target responses by class',
            ['2xx', '3xx', '4xx', '5xx'].map((c) => sum(`aws.applicationelb.httpcode_target_${c}_count`)),
            { alias: ['2xx', '3xx', '4xx', '5xx'], display: 'bars' },
          ),
          ts(
            'Load balancer errors',
            ['4xx', '502', '503', '504'].map((c) => sum(`aws.applicationelb.httpcode_elb_${c}_count`)),
            { alias: ['4xx', '502', '503', '504'], display: 'bars', palette: 'warm' },
          ),
          top('Most 5xx (targets and load balancer)', [sum('aws.applicationelb.httpcode_target_5xx_count', 'loadbalancer'), sum('aws.applicationelb.httpcode_elb_5xx_count', 'loadbalancer')], {
            formula: 'query1 + query2',
            palette: 'warm',
          }),
          ts('Target connection errors', sum('aws.applicationelb.target_connection_error_count', 'loadbalancer'), { display: 'bars', palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/NetworkELB': {
    vars: ['loadbalancer', 'targetgroup'],
    monitor: 'amazon_elb',
    note: 'Network Load Balancers: healthy hosts per target group, TCP, TLS and UDP flows and bytes, the resets sent by clients, targets or the load balancer, and TLS negotiation errors.',
    units: {
      ProcessedBytes: 'byte',
      ProcessedBytes_TCP: 'byte',
      ProcessedBytes_TLS: 'byte',
      ProcessedBytes_UDP: 'byte',
      HealthyHostCount: 'host',
      UnHealthyHostCount: 'host',
      ProcessedPackets: 'packet',
    },
    build: ({ sum, avg, max }) => ({
      kpis: [
        qv('Healthy hosts', sum('aws.networkelb.healthy_host_count'), { agg: 'min', good: true }),
        qv('Unhealthy hosts', sum('aws.networkelb.un_healthy_host_count'), { agg: 'max', bad: true }),
        qv('Active flows', sum('aws.networkelb.active_flow_count'), { agg: 'avg' }),
        qv('New flows', sum('aws.networkelb.new_flow_count')),
        qv('Processed bytes', sum('aws.networkelb.processed_bytes')),
        qv('Resets by the load balancer', sum('aws.networkelb.tcp_elb_reset_count'), { bad: true }),
      ],
      groups: [
        group('Host health', [
          ts('Healthy hosts by target group', sum('aws.networkelb.healthy_host_count', 'targetgroup'), { palette: 'green' }),
          ts('Unhealthy hosts by target group', sum('aws.networkelb.un_healthy_host_count', 'targetgroup'), { display: 'bars', palette: 'warm' }),
          top('Load balancers with unhealthy hosts', max('aws.networkelb.un_healthy_host_count', 'loadbalancer'), { w: 12, palette: 'warm' }),
        ]),
        group('Flows', [
          ts(
            'Active flows by protocol',
            ['tcp', 'tls', 'udp'].map((p) => sum(`aws.networkelb.active_flow_count_${p}`)),
            { alias: ['TCP', 'TLS', 'UDP'] },
          ),
          ts(
            'New flows by protocol',
            ['tcp', 'tls', 'udp'].map((p) => sum(`aws.networkelb.new_flow_count_${p}`)),
            { alias: ['TCP', 'TLS', 'UDP'], display: 'bars' },
          ),
          top('Most active flows', avg('aws.networkelb.active_flow_count', 'loadbalancer')),
          ts('Peak packets per second', sum('aws.networkelb.peak_packets_per_second', 'loadbalancer')),
        ]),
        group('Traffic', [
          ts(
            'Bytes by protocol',
            ['tcp', 'tls', 'udp'].map((p) => sum(`aws.networkelb.processed_bytes_${p}`)),
            { alias: ['TCP', 'TLS', 'UDP'], display: 'bars' },
          ),
          ts('Packets by load balancer', sum('aws.networkelb.processed_packets', 'loadbalancer')),
          ts('Capacity units consumed (LCUs)', sum('aws.networkelb.consumed_lcus', 'loadbalancer'), { w: 12 }),
        ]),
        group('Resets and errors', [
          ts('TCP resets by origin', [sum('aws.networkelb.tcp_client_reset_count'), sum('aws.networkelb.tcp_target_reset_count'), sum('aws.networkelb.tcp_elb_reset_count')], {
            alias: ['client', 'target', 'load balancer'],
            palette: 'warm',
          }),
          ts('TLS negotiation errors', [sum('aws.networkelb.client_tlsnegotiation_error_count'), sum('aws.networkelb.target_tlsnegotiation_error_count')], {
            alias: ['client', 'target'],
            palette: 'warm',
          }),
          ts('Port allocation errors', sum('aws.networkelb.port_allocation_error_count', 'loadbalancer'), { display: 'bars', palette: 'warm', w: 12 }),
        ]),
      ],
    }),
  },

  'AWS/ELB': {
    vars: ['loadbalancername'],
    monitor: 'amazon_elb',
    note: 'Classic Load Balancers: healthy and unhealthy instances, requests and latency, the responses of the instances behind them and the errors of the load balancer, and its surge queue.',
    logs: [['Load balancer access logs', 'source:elb']],
    units: { Latency: 'second', RequestCount: 'request', HealthyHostCount: 'host', UnHealthyHostCount: 'host' },
    build: ({ sum, avg, max }) => ({
      kpis: [
        qv('Healthy instances', sum('aws.elb.healthy_host_count'), { agg: 'min', good: true }),
        qv('Unhealthy instances', sum('aws.elb.un_healthy_host_count'), { agg: 'max', bad: true }),
        qv('Requests', sum('aws.elb.request_count')),
        qv('Avg latency', avg('aws.elb.latency'), { precision: 3 }),
        qv('Backend 5xx', sum('aws.elb.httpcode_backend_5xx'), { bad: true }),
        qv('Longest surge queue', max('aws.elb.surge_queue_length.maximum')),
      ],
      groups: [
        group('Instance health', [
          ts('Healthy instances', sum('aws.elb.healthy_host_count', 'loadbalancername'), { palette: 'green' }),
          ts('Unhealthy instances', sum('aws.elb.un_healthy_host_count', 'loadbalancername'), { display: 'bars', palette: 'warm' }),
        ]),
        group('Traffic and latency', [
          ts('Requests by load balancer', sum('aws.elb.request_count', 'loadbalancername'), { display: 'bars' }),
          ts('Latency', [avg('aws.elb.latency.p50'), avg('aws.elb.latency.p95'), avg('aws.elb.latency.p99')], { alias: ['p50', 'p95', 'p99'] }),
          top('Busiest load balancers', sum('aws.elb.request_count', 'loadbalancername')),
          top('Slowest load balancers (p95)', avg('aws.elb.latency.p95', 'loadbalancername'), { palette: 'orange' }),
        ]),
        group('Responses and errors', [
          ts(
            'Backend responses by class',
            ['2xx', '3xx', '4xx', '5xx'].map((c) => sum(`aws.elb.httpcode_backend_${c}`)),
            { alias: ['2xx', '3xx', '4xx', '5xx'], display: 'bars' },
          ),
          ts('Load balancer errors', [sum('aws.elb.httpcode_elb_4xx'), sum('aws.elb.httpcode_elb_5xx')], { alias: ['4xx', '5xx'], display: 'bars', palette: 'warm' }),
          ts('Backend connection errors', sum('aws.elb.backend_connection_errors', 'loadbalancername'), { palette: 'warm' }),
          ts('Surge queue and spillover', [max('aws.elb.surge_queue_length.maximum'), sum('aws.elb.spillover_count')], { alias: ['surge queue', 'spillover'], palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/Lambda': {
    vars: ['functionname'],
    monitor: 'amazon_lambda',
    note: 'Functions: invocations, how long they run (p95 and the longest run, to compare with their timeout), errors and throttles, and concurrency. Traces, cold starts and memory come from the Datadog Lambda extension.',
    logs: [
      ['Lambda logs', 'source:lambda'],
      ['Lambda errors', 'source:lambda status:error'],
    ],
    units: {
      Duration: 'millisecond',
      IteratorAge: 'millisecond',
      PostRuntimeExtensionsDuration: 'millisecond',
      Invocations: 'invocation',
      Errors: 'error',
      Throttles: 'throttle',
      ProvisionedConcurrencyInvocations: 'invocation',
      ProvisionedConcurrencySpilloverInvocations: 'invocation',
    },
    build: ({ sum, avg, max }) => ({
      kpis: [
        qv('Invocations', sum('aws.lambda.invocations')),
        qv('Errors', sum('aws.lambda.errors'), { bad: true }),
        qv('Error rate', [sum('aws.lambda.errors'), sum('aws.lambda.invocations')], { formula: '100 * query1 / query2', precision: 2, unit: '%' }),
        qv('Duration p95', avg('aws.lambda.duration.p95')),
        qv('Throttles', sum('aws.lambda.throttles'), { bad: true }),
        qv('Peak concurrency', max('aws.lambda.concurrent_executions.maximum')),
      ],
      groups: [
        group('Invocations', [
          ts('Invocations by function', sum('aws.lambda.invocations', 'functionname'), { display: 'bars', w: 8 }),
          top('Most invoked', sum('aws.lambda.invocations', 'functionname'), { w: 4 }),
          ts('Provisioned concurrency invocations', [sum('aws.lambda.provisioned_concurrency_invocations'), sum('aws.lambda.provisioned_concurrency_spillover_invocations')], {
            alias: ['provisioned', 'spillover'],
            w: 12,
          }),
        ]),
        group('Execution time', [
          ts('Duration p95 by function', avg('aws.lambda.duration.p95', 'functionname')),
          ts('Duration percentiles and longest run', [avg('aws.lambda.duration.p50'), avg('aws.lambda.duration.p95'), avg('aws.lambda.duration.p99'), max('aws.lambda.duration.maximum')], {
            alias: ['p50', 'p95', 'p99', 'max'],
          }),
          top('Slowest functions (p95)', avg('aws.lambda.duration.p95', 'functionname'), { w: 4, palette: 'orange' }),
          top('Longest single run', max('aws.lambda.duration.maximum', 'functionname'), { w: 4, palette: 'orange' }),
          top('Total time spent', sum('aws.lambda.duration.sum', 'functionname'), { w: 4 }),
        ]),
        group('Errors and throttles', [
          ts('Errors by function', sum('aws.lambda.errors', 'functionname'), { display: 'bars', palette: 'warm' }),
          ts('Error rate by function', [sum('aws.lambda.errors', 'functionname'), sum('aws.lambda.invocations', 'functionname')], {
            formulas: [['100 * query1 / query2', 'error rate']],
            palette: 'warm',
          }),
          top('Most errors', sum('aws.lambda.errors', 'functionname'), { w: 4, palette: 'warm' }),
          ts('Throttles by function', sum('aws.lambda.throttles', 'functionname'), { display: 'bars', palette: 'warm', w: 4 }),
          ts('Asynchronous delivery failures', [sum('aws.lambda.dead_letter_errors'), sum('aws.lambda.destination_delivery_failures')], {
            alias: ['dead-letter queue', 'destinations'],
            palette: 'warm',
            w: 4,
          }),
        ]),
        group('Concurrency', [
          ts('Concurrent executions by function', max('aws.lambda.concurrent_executions.maximum', 'functionname')),
          ts('Unreserved concurrency (account)', max('aws.lambda.unreserved_concurrent_executions.maximum'), { alias: ['unreserved'] }),
          ts('Provisioned concurrency used', avg('aws.lambda.provisioned_concurrency_utilization', 'functionname')),
          ts('Stream iterator age', max('aws.lambda.iterator_age.maximum', 'functionname')),
        ]),
      ],
    }),
  },

  'AWS/DynamoDB': {
    vars: ['tablename'],
    monitor: 'amazon_dynamodb',
    note: 'Tables: read and write capacity consumed against what is provisioned, throttling, request latency and errors per operation, and global table replication.',
    units: { SuccessfulRequestLatency: 'millisecond', ReplicationLatency: 'millisecond', AccountProvisionedReadCapacityUtilization: 'percent', AccountProvisionedWriteCapacityUtilization: 'percent' },
    build: ({ sum, avg }) => ({
      kpis: [
        qv('Read capacity consumed', sum('aws.dynamodb.consumed_read_capacity_units')),
        qv('Write capacity consumed', sum('aws.dynamodb.consumed_write_capacity_units')),
        qv('Throttled requests', sum('aws.dynamodb.throttled_requests'), { bad: true }),
        qv('System errors', sum('aws.dynamodb.system_errors'), { bad: true }),
        qv('User errors', sum('aws.dynamodb.user_errors'), { bad: true }),
        qv('Avg request latency', avg('aws.dynamodb.successful_request_latency'), { precision: 1 }),
      ],
      groups: [
        group('Capacity', [
          ts('Reads: consumed and provisioned (per second)', [sum('aws.dynamodb.consumed_read_capacity_units'), sum('aws.dynamodb.provisioned_read_capacity_units')], {
            formulas: [
              ['query1 / 60', 'consumed'],
              ['query2', 'provisioned'],
            ],
          }),
          ts('Writes: consumed and provisioned (per second)', [sum('aws.dynamodb.consumed_write_capacity_units'), sum('aws.dynamodb.provisioned_write_capacity_units')], {
            formulas: [
              ['query1 / 60', 'consumed'],
              ['query2', 'provisioned'],
            ],
          }),
          top('Read capacity used (%)', [sum('aws.dynamodb.consumed_read_capacity_units', 'tablename'), sum('aws.dynamodb.provisioned_read_capacity_units', 'tablename')], {
            formula: '100 * query1 / 60 / query2',
            palette: 'orange',
          }),
          top('Write capacity used (%)', [sum('aws.dynamodb.consumed_write_capacity_units', 'tablename'), sum('aws.dynamodb.provisioned_write_capacity_units', 'tablename')], {
            formula: '100 * query1 / 60 / query2',
            palette: 'orange',
          }),
        ]),
        group('Throttling', [
          ts('Read throttle events by table', sum('aws.dynamodb.read_throttle_events', 'tablename'), { display: 'bars', palette: 'warm', w: 4 }),
          ts('Write throttle events by table', sum('aws.dynamodb.write_throttle_events', 'tablename'), { display: 'bars', palette: 'warm', w: 4 }),
          ts('Throttled requests by operation', sum('aws.dynamodb.throttled_requests', 'operation'), { palette: 'warm', w: 4 }),
        ]),
        group('Latency and errors', [
          ts('Request latency by operation', avg('aws.dynamodb.successful_request_latency', 'operation')),
          top('Slowest tables', avg('aws.dynamodb.successful_request_latency', 'tablename'), { palette: 'orange' }),
          ts('System errors by operation', sum('aws.dynamodb.system_errors', 'operation'), { palette: 'warm', w: 4 }),
          ts('Conditional check failures', sum('aws.dynamodb.conditional_check_failed_requests', 'tablename'), { w: 4 }),
          ts('Transaction conflicts', sum('aws.dynamodb.transaction_conflict.sum', 'tablename'), { w: 4 }),
        ]),
        group('Items and replication', [
          ts('Items returned by operation', sum('aws.dynamodb.returned_item_count', 'operation')),
          ts('Replication latency by region', avg('aws.dynamodb.replication_latency', 'receivingregion')),
          ts('Items deleted by TTL', sum('aws.dynamodb.time_to_live_deleted_item_count', 'tablename'), { display: 'bars' }),
          ts('Account provisioned capacity used', [avg('aws.dynamodb.account_provisioned_read_capacity_utilization'), avg('aws.dynamodb.account_provisioned_write_capacity_utilization')], {
            alias: ['reads', 'writes'],
          }),
        ]),
      ],
    }),
  },

  'AWS/ElastiCache': {
    vars: ['cacheclusterid'],
    monitor: 'amazon_elasticache',
    note: 'Redis OSS and Valkey nodes (Memcached nodes share the host metrics): hits and command latency, memory and evictions, connections, and the host behind each node.',
    units: {
      CPUUtilization: 'percent',
      EngineCPUUtilization: 'percent',
      CacheHitRate: 'percent',
      DatabaseMemoryUsagePercentage: 'percent',
      FreeableMemory: 'byte',
      SwapUsage: 'byte',
      NetworkBytesIn: 'byte',
      NetworkBytesOut: 'byte',
      ReplicationLag: 'second',
      GetTypeCmdsLatency: 'microsecond',
      SetTypeCmdsLatency: 'microsecond',
      KeyBasedCmdsLatency: 'microsecond',
      StringBasedCmdsLatency: 'microsecond',
    },
    build: ({ sum, avg, max }) => ({
      kpis: [
        qv('Engine CPU', avg('aws.elasticache.engine_cpuutilization'), { precision: 1 }),
        qv('Hit rate', avg('aws.elasticache.cache_hit_rate'), { precision: 1 }),
        qv('Memory used', avg('aws.elasticache.database_memory_usage_percentage'), { precision: 1 }),
        qv('Connections', sum('aws.elasticache.curr_connections.maximum'), { agg: 'avg' }),
        qv('Evictions', sum('aws.elasticache.evictions'), { bad: true }),
        qv('Replication lag', max('aws.elasticache.replication_lag'), { precision: 2 }),
      ],
      groups: [
        group('Performance', [
          ts('Hits and misses', [sum('aws.elasticache.cache_hits'), sum('aws.elasticache.cache_misses')], { alias: ['hits', 'misses'], display: 'bars' }),
          ts('Hit rate by node', avg('aws.elasticache.cache_hit_rate', 'cacheclusterid')),
          ts(
            'Command latency by type',
            ['get_type', 'set_type', 'key_based', 'string_based'].map((t) => avg(`aws.elasticache.${t}_cmds_latency`)),
            { alias: ['get', 'set', 'key', 'string'] },
          ),
          ts(
            'Commands by type',
            ['get_type', 'set_type', 'key_based', 'hash_based', 'list_based'].map((t) => sum(`aws.elasticache.${t}_cmds`)),
            { alias: ['get', 'set', 'key', 'hash', 'list'] },
          ),
        ]),
        group('Memory', [
          ts('Memory used (%) by node', avg('aws.elasticache.database_memory_usage_percentage', 'cacheclusterid')),
          top('Least freeable memory', avg('aws.elasticache.freeable_memory', 'cacheclusterid'), { order: 'asc' }),
          ts('Evictions and reclaimed keys', [sum('aws.elasticache.evictions'), sum('aws.elasticache.reclaimed')], { alias: ['evicted', 'expired'], palette: 'warm' }),
          ts('Fragmentation ratio', avg('aws.elasticache.memory_fragmentation_ratio', 'cacheclusterid')),
        ]),
        group('Connections', [
          ts('Current connections by node', max('aws.elasticache.curr_connections.maximum', 'cacheclusterid')),
          ts('New connections by node', sum('aws.elasticache.new_connections', 'cacheclusterid'), { display: 'bars' }),
          ts(
            'Authentication and authorization failures',
            [sum('aws.elasticache.authentication_failures'), sum('aws.elasticache.command_authorization_failures'), sum('aws.elasticache.key_authorization_failures')],
            { alias: ['authentication', 'commands', 'keys'], palette: 'warm', w: 12 },
          ),
        ]),
        group('Hosts', [
          ts('Engine CPU by node', avg('aws.elasticache.engine_cpuutilization', 'cacheclusterid')),
          ts('Host CPU by node', avg('aws.elasticache.cpuutilization', 'cacheclusterid')),
          ts('Network in and out', [sum('aws.elasticache.network_bytes_in.sum'), sum('aws.elasticache.network_bytes_out.sum')], {
            formulas: [
              ['query1', 'in'],
              ['0 - query2', 'out'],
            ],
            display: 'bars',
          }),
          ts('Replication lag by node', avg('aws.elasticache.replication_lag', 'cacheclusterid')),
          ts('Swap used by node', avg('aws.elasticache.swap_usage', 'cacheclusterid')),
          ts(
            'Network allowances exceeded',
            [
              sum('aws.elasticache.network_bandwidth_in_allowance_exceeded.sum'),
              sum('aws.elasticache.network_bandwidth_out_allowance_exceeded.sum'),
              sum('aws.elasticache.network_packets_per_second_allowance_exceeded.sum'),
            ],
            { alias: ['bandwidth in', 'bandwidth out', 'packets per second'], palette: 'warm' },
          ),
        ]),
      ],
    }),
  },

  'AWS/SQS': {
    vars: ['queuename'],
    monitor: 'amazon_sqs',
    note: 'Queues: messages waiting, in flight and delayed, what producers send and consumers receive and delete, how old the oldest message is, and the dead-letter queues collecting what failed.',
    units: { ApproximateAgeOfOldestMessage: 'second', SentMessageSize: 'byte', NumberOfMessagesSent: 'message', NumberOfMessagesReceived: 'message', NumberOfMessagesDeleted: 'message' },
    build: ({ sum, avg, max }) => ({
      kpis: [
        qv('Messages sent', sum('aws.sqs.number_of_messages_sent')),
        qv('Messages received', sum('aws.sqs.number_of_messages_received')),
        qv('Messages deleted', sum('aws.sqs.number_of_messages_deleted')),
        qv('Empty receives', sum('aws.sqs.number_of_empty_receives')),
        qv('Messages waiting', sum('aws.sqs.approximate_number_of_messages_visible'), { agg: 'last' }),
        qv('Oldest message (s)', max('aws.sqs.approximate_age_of_oldest_message')),
      ],
      groups: [
        group('Message activity', [
          ts(
            'Messages by state',
            [sum('aws.sqs.approximate_number_of_messages_visible'), sum('aws.sqs.approximate_number_of_messages_not_visible'), sum('aws.sqs.approximate_number_of_messages_delayed')],
            { alias: ['waiting', 'in flight', 'delayed'], display: 'area', w: 8 },
          ),
          ts('Sent, received and deleted', [sum('aws.sqs.number_of_messages_sent'), sum('aws.sqs.number_of_messages_received'), sum('aws.sqs.number_of_messages_deleted')], {
            alias: ['sent', 'received', 'deleted'],
            w: 4,
          }),
        ]),
        group('Queues', [
          top('Most messages sent', sum('aws.sqs.number_of_messages_sent', 'queuename'), { w: 4 }),
          top('Longest backlogs', avg('aws.sqs.approximate_number_of_messages_visible', 'queuename'), { w: 4, palette: 'orange' }),
          top('Oldest messages', max('aws.sqs.approximate_age_of_oldest_message', 'queuename'), { w: 4, palette: 'orange' }),
          ts('Age of the oldest message by queue', max('aws.sqs.approximate_age_of_oldest_message', 'queuename')),
          ts('Messages sent by queue', sum('aws.sqs.number_of_messages_sent', 'queuename'), { display: 'bars' }),
        ]),
        group('Consumers', [
          ts('Backlog growth (sent − deleted)', [sum('aws.sqs.number_of_messages_sent'), sum('aws.sqs.number_of_messages_deleted')], { formulas: [['query1 - query2', 'growth']], display: 'bars' }),
          ts('Messages in flight by queue', avg('aws.sqs.approximate_number_of_messages_not_visible', 'queuename')),
          ts('Empty receives by queue', sum('aws.sqs.number_of_empty_receives', 'queuename'), { display: 'bars' }),
          ts('Message size by queue', avg('aws.sqs.sent_message_size', 'queuename')),
        ]),
        group('Dead-letter queues', [
          txt('Queues whose name contains `dlq`: messages that failed processing too many times. Anything waiting here needs a look.', { w: 4 }),
          top('Messages waiting in dead-letter queues', avg('aws.sqs.approximate_number_of_messages_visible', 'queuename', 'queuename:*dlq*'), { w: 4, palette: 'warm' }),
          ts('Messages arriving in dead-letter queues', sum('aws.sqs.number_of_messages_sent', 'queuename', 'queuename:*dlq*'), { display: 'bars', palette: 'warm', w: 4 }),
        ]),
      ],
    }),
  },

  'AWS/SNS': {
    vars: ['topicname'],
    monitor: 'amazon_sns',
    note: 'Topics: messages published, notifications delivered to subscribers or failed, and the SMS messages sent with what they cost this month.',
    units: { PublishSize: 'byte', NumberOfMessagesPublished: 'message', NumberOfNotificationsDelivered: 'message', NumberOfNotificationsFailed: 'message', SMSMonthToDateSpentUSD: 'dollar' },
    build: ({ sum, avg, max }) => ({
      kpis: [
        qv('Published', sum('aws.sns.number_of_messages_published')),
        qv('Delivered', sum('aws.sns.number_of_notifications_delivered'), { good: true }),
        qv('Failed', sum('aws.sns.number_of_notifications_failed'), { bad: true }),
        qv('Delivery success', [sum('aws.sns.number_of_notifications_delivered'), sum('aws.sns.number_of_notifications_failed')], {
          formula: '100 * query1 / (query1 + query2)',
          precision: 2,
          unit: '%',
        }),
        qv('Avg message size', avg('aws.sns.publish_size')),
        qv('SMS spend this month', max('aws.sns.smsmonth_to_date_spent_usd.maximum'), { precision: 2 }),
      ],
      groups: [
        group('Publishing', [
          ts('Messages published by topic', sum('aws.sns.number_of_messages_published', 'topicname'), { display: 'bars', w: 8 }),
          top('Busiest topics', sum('aws.sns.number_of_messages_published', 'topicname'), { w: 4 }),
          ts('Message size by topic', avg('aws.sns.publish_size', 'topicname'), { w: 12 }),
        ]),
        group('Delivery', [
          ts('Delivered and failed', [sum('aws.sns.number_of_notifications_delivered'), sum('aws.sns.number_of_notifications_failed')], { alias: ['delivered', 'failed'], display: 'bars' }),
          ts('Failures by topic', sum('aws.sns.number_of_notifications_failed', 'topicname'), { palette: 'warm' }),
          top('Topics with most failures', sum('aws.sns.number_of_notifications_failed', 'topicname'), { palette: 'warm' }),
          ts('Delivery success by topic', [sum('aws.sns.number_of_notifications_delivered', 'topicname'), sum('aws.sns.number_of_notifications_failed', 'topicname')], {
            formulas: [['100 * query1 / (query1 + query2)', 'success %']],
          }),
        ]),
        group('SMS', [
          ts('SMS spend this month', max('aws.sns.smsmonth_to_date_spent_usd.maximum'), { w: 4 }),
          ts('SMS success rate by country', avg('aws.sns.smssuccess_rate', 'country'), { w: 4 }),
          ts('Messages published to phone numbers', sum('aws.sns.phone_number_direct'), { display: 'bars', w: 4 }),
        ]),
      ],
    }),
  },

  'AWS/ApiGateway': {
    vars: ['apiid', 'stage'],
    monitor: 'amazon_api_gateway',
    note: 'REST, HTTP and WebSocket APIs: requests, latency (p95 and p99) and how much of it the backend takes, client and server errors, and the stage cache.',
    logs: [['API Gateway logs', 'source:apigateway']],
    units: { Latency: 'millisecond', IntegrationLatency: 'millisecond', DataProcessed: 'byte', Count: 'request' },
    build: ({ sum, avg }) => ({
      kpis: [
        qv('Requests', sum('aws.apigateway.count')),
        qv('4xx errors', [sum('aws.apigateway.4xxerror'), sum('aws.apigateway.4xx')], { formula: 'query1 + query2', bad: true }),
        qv('5xx errors', [sum('aws.apigateway.5xxerror'), sum('aws.apigateway.5xx')], { formula: 'query1 + query2', bad: true }),
        qv('Avg latency', avg('aws.apigateway.latency')),
        qv('Latency p95', avg('aws.apigateway.latency.p95')),
        qv('Cache hit rate', [sum('aws.apigateway.cache_hit_count'), sum('aws.apigateway.cache_miss_count')], { formula: '100 * query1 / (query1 + query2)', precision: 1, unit: '%' }),
      ],
      groups: [
        group('Traffic', [
          ts('Requests by API', sum('aws.apigateway.count', 'apiid'), { display: 'bars', w: 8 }),
          top('Busiest APIs', sum('aws.apigateway.count', 'apiid'), { w: 4 }),
          ts('Data processed (HTTP APIs)', sum('aws.apigateway.data_processed', 'apiid')),
          ts('WebSocket connections and messages', [sum('aws.apigateway.connect_count'), sum('aws.apigateway.message_count')], { alias: ['connects', 'messages'] }),
        ]),
        group('Latency', [
          ts('Latency percentiles', [avg('aws.apigateway.latency.p50'), avg('aws.apigateway.latency.p95'), avg('aws.apigateway.latency.p99')], { alias: ['p50', 'p95', 'p99'] }),
          ts('Integration latency p95 by API', avg('aws.apigateway.integration_latency.p95', 'apiid')),
          top('Slowest APIs (p95)', avg('aws.apigateway.latency.p95', 'apiid'), { palette: 'orange' }),
          ts('Time spent in API Gateway (latency − integration)', [avg('aws.apigateway.latency'), avg('aws.apigateway.integration_latency')], { formulas: [['query1 - query2', 'overhead']] }),
        ]),
        group('Errors', [
          ts('Client errors (4xx) by API', sum('aws.apigateway.4xxerror', 'apiid'), { display: 'bars', palette: 'warm', w: 4 }),
          ts('Server errors (5xx) by API', sum('aws.apigateway.5xxerror', 'apiid'), { display: 'bars', palette: 'warm', w: 4 }),
          ts('HTTP API errors', [sum('aws.apigateway.4xx'), sum('aws.apigateway.5xx')], { alias: ['4xx', '5xx'], palette: 'warm', w: 4 }),
          ts('WebSocket errors', [sum('aws.apigateway.client_error'), sum('aws.apigateway.execution_error'), sum('aws.apigateway.integration_error')], {
            alias: ['client', 'execution', 'integration'],
            palette: 'warm',
          }),
          top('APIs with most 5xx', sum('aws.apigateway.5xxerror', 'apiid'), { palette: 'warm' }),
        ]),
        group('Cache', [
          ts('Cache hits and misses', [sum('aws.apigateway.cache_hit_count'), sum('aws.apigateway.cache_miss_count')], { alias: ['hits', 'misses'], display: 'bars' }),
          ts('Cache hit rate by stage', [sum('aws.apigateway.cache_hit_count', 'stage'), sum('aws.apigateway.cache_miss_count', 'stage')], {
            formulas: [['100 * query1 / (query1 + query2)', 'hit rate']],
          }),
        ]),
      ],
    }),
  },

  'AWS/CloudFront': {
    vars: ['distributionid'],
    monitor: 'amazon_cloudfront',
    note: 'Distributions: requests and bytes served from the edge, error rates by status code, the cache hit rate and origin latency (additional metrics, enabled per distribution), and the functions running at the edge.',
    logs: [['CloudFront access logs', 'source:cloudfront']],
    build: ({ sum, avg }) => {
      const d = 'distributionid:*';
      const fn = 'functionname:*';
      return {
        kpis: [
          qv('Requests', sum('aws.cloudfront.requests', null, d)),
          qv('Error rate', avg('aws.cloudfront.total_error_rate', null, d), { precision: 2 }),
          qv('5xx error rate', avg('aws.cloudfront.5xx_error_rate', null, d), { precision: 2 }),
          qv('Cache hit rate', avg('aws.cloudfront.cache_hit_rate', null, d), { precision: 1 }),
          qv('Bytes downloaded', sum('aws.cloudfront.bytes_downloaded', null, d)),
          qv('Origin latency', avg('aws.cloudfront.origin_latency', null, d)),
        ],
        groups: [
          group('Traffic', [
            ts('Requests by distribution', sum('aws.cloudfront.requests', 'distributionid', d), { display: 'bars', w: 8 }),
            top('Busiest distributions', sum('aws.cloudfront.requests', 'distributionid', d), { w: 4 }),
            ts('Downloaded and uploaded', [sum('aws.cloudfront.bytes_downloaded', null, d), sum('aws.cloudfront.bytes_uploaded', null, d)], {
              formulas: [
                ['query1', 'downloaded'],
                ['0 - query2', 'uploaded'],
              ],
              display: 'bars',
            }),
            top('Most data served', sum('aws.cloudfront.bytes_downloaded', 'distributionid', d)),
          ]),
          group('Errors', [
            ts('Error rates', [avg('aws.cloudfront.total_error_rate', null, d), avg('aws.cloudfront.4xx_error_rate', null, d), avg('aws.cloudfront.5xx_error_rate', null, d)], {
              alias: ['total', '4xx', '5xx'],
              palette: 'warm',
            }),
            ts('5xx error rate by distribution', avg('aws.cloudfront.5xx_error_rate', 'distributionid', d), { palette: 'warm' }),
            ts(
              'Origin errors (502, 503, 504)',
              ['502', '503', '504'].map((c) => avg(`aws.cloudfront.${c}_error_rate`, null, d)),
              { alias: ['502', '503', '504'], palette: 'warm' },
            ),
            ts(
              'Refused and missing (401, 403, 404)',
              ['401', '403', '404'].map((c) => avg(`aws.cloudfront.${c}_error_rate`, null, d)),
              { alias: ['401', '403', '404'] },
            ),
          ]),
          group('Cache and origin', [
            ts('Cache hit rate by distribution', avg('aws.cloudfront.cache_hit_rate', 'distributionid', d)),
            ts('Origin latency by distribution', avg('aws.cloudfront.origin_latency', 'distributionid', d)),
            top('Lowest cache hit rate', avg('aws.cloudfront.cache_hit_rate', 'distributionid', d), { order: 'asc' }),
            top('Slowest origins', avg('aws.cloudfront.origin_latency', 'distributionid', d), { palette: 'orange' }),
          ]),
          group('Edge functions', [
            ts(
              'Lambda@Edge errors',
              [sum('aws.cloudfront.lambda_execution_error', null, d), sum('aws.cloudfront.lambda_validation_error', null, d), sum('aws.cloudfront.lambda_limit_exceeded_error', null, d)],
              { alias: ['execution', 'validation', 'limit exceeded'], palette: 'warm' },
            ),
            ts('CloudFront Functions invocations', sum('aws.cloudfront.function_invocations', 'functionname', fn), { display: 'bars' }),
            ts(
              'Function errors and throttles',
              [sum('aws.cloudfront.function_execution_errors.sum', null, fn), sum('aws.cloudfront.function_validation_errors.sum', null, fn), sum('aws.cloudfront.function_throttles', null, fn)],
              {
                alias: ['execution', 'validation', 'throttles'],
                palette: 'warm',
              },
            ),
            ts('Function compute utilization', avg('aws.cloudfront.function_compute_utilization', 'functionname', fn)),
          ]),
        ],
      };
    },
  },

  'AWS/ECS': {
    vars: ['clustername', 'servicename'],
    monitor: 'amazon_ecs',
    note: 'Clusters and services: CPU and memory used against what is reserved, and the traffic of services using Service Connect. Tasks and containers in more detail come from Container Insights or the Agent.',
    build: ({ sum, avg, max }) => {
      const s = 'servicename:*';
      const c = 'clustername:*';
      return {
        kpis: [
          qv('Cluster CPU', avg('aws.ecs.cluster.cpuutilization', null, c), { precision: 1 }),
          qv('Cluster memory', avg('aws.ecs.cluster.memory_utilization', null, c), { precision: 1 }),
          qv('CPU reserved', avg('aws.ecs.cluster.cpureservation', null, c), { precision: 1 }),
          qv('Memory reserved', avg('aws.ecs.cluster.memory_reservation', null, c), { precision: 1 }),
          qv('Busiest service CPU', max('aws.ecs.service.cpuutilization', null, s), { precision: 1 }),
        ],
        groups: [
          group('Clusters', [
            ts('CPU by cluster', avg('aws.ecs.cluster.cpuutilization', 'clustername', c)),
            ts('Memory by cluster', avg('aws.ecs.cluster.memory_utilization', 'clustername', c)),
            ts('CPU and memory reserved', [avg('aws.ecs.cluster.cpureservation', null, c), avg('aws.ecs.cluster.memory_reservation', null, c)], { alias: ['CPU', 'memory'] }),
            ts('GPU reserved by cluster', avg('aws.ecs.cluster.gpureservation', 'clustername', c)),
          ]),
          group('Services', [
            top('Most CPU', avg('aws.ecs.service.cpuutilization', 'servicename', s), { palette: 'orange' }),
            top('Most memory', avg('aws.ecs.service.memory_utilization', 'servicename', s), { palette: 'orange' }),
            ts('CPU by service', avg('aws.ecs.service.cpuutilization', 'servicename', s)),
            ts('Memory by service', avg('aws.ecs.service.memory_utilization', 'servicename', s)),
            ts('EBS filesystem used by service', avg('aws.ecs.service.ebsfilesystem_utilization', 'servicename', s), { w: 12 }),
          ]),
          group('Service Connect traffic', [
            ts('Requests by service', sum('aws.ecs.service.request_count', 'servicename', s), { display: 'bars' }),
            ts('Response time by service', avg('aws.ecs.service.target_response_time', 'servicename', s)),
            ts(
              'Responses by class',
              ['2', '4', '5'].map((k) => sum(`aws.ecs.service.httpcode_target_${k}xxcount`, null, s)),
              { alias: ['2xx', '4xx', '5xx'], display: 'bars' },
            ),
            ts('Connections', [sum('aws.ecs.service.active_connection_count', null, s), sum('aws.ecs.service.new_connection_count', null, s)], { alias: ['active', 'new'] }),
            ts('Bytes processed by service', sum('aws.ecs.service.processed_bytes', 'servicename', s)),
            ts('TLS negotiation errors', sum('aws.ecs.service.client_tlsnegotiation_error_count', 'servicename', s), { palette: 'warm' }),
          ]),
        ],
      };
    },
  },

  'ECS/ContainerInsights': {
    vars: ['clustername', 'servicename'],
    monitor: 'amazon_ecs',
    note: 'Container Insights for ECS: running, desired and pending tasks per service, CPU and memory used against reserved (per service and per task definition family), network, storage and restarts.',
    build: ({ sum, avg }) => {
      const s = 'servicename:*';
      const c = 'clustername:*';
      const fam = 'taskdefinitionfamily:*';
      return {
        kpis: [
          qv('Services', sum('ecs.containerinsights.service_count', null, c), { agg: 'last' }),
          qv('Running tasks', sum('ecs.containerinsights.running_task_count', null, s), { agg: 'last', good: true }),
          qv('Desired tasks', sum('ecs.containerinsights.desired_task_count', null, s), { agg: 'last' }),
          qv('Pending tasks', sum('ecs.containerinsights.pending_task_count', null, s), { agg: 'last', bad: true }),
          qv('Container instances', sum('ecs.containerinsights.container_instance_count', null, c), { agg: 'last' }),
          qv('Restarts', sum('ecs.containerinsights.restart_count', null, s), { bad: true }),
        ],
        groups: [
          group('Tasks and deployments', [
            ts('Running and desired tasks', [sum('ecs.containerinsights.running_task_count', null, s), sum('ecs.containerinsights.desired_task_count', null, s)], { alias: ['running', 'desired'] }),
            ts('Running tasks by service', sum('ecs.containerinsights.running_task_count', 'servicename', s)),
            ts('Pending tasks by service', sum('ecs.containerinsights.pending_task_count', 'servicename', s), { display: 'bars', palette: 'warm' }),
            ts('Deployments in progress', sum('ecs.containerinsights.deployment_count', 'servicename', s), { display: 'bars' }),
          ]),
          group('CPU and memory', [
            ts('Task CPU by service (%)', avg('ecs.containerinsights.task_cpu_utilization', 'servicename', s)),
            ts('Task memory by service (%)', avg('ecs.containerinsights.task_memory_utilization', 'servicename', s)),
            top('CPU used of reserved', [sum('ecs.containerinsights.cpu_utilized', 'servicename', s), sum('ecs.containerinsights.cpu_reserved', 'servicename', s)], {
              formula: '100 * query1 / query2',
              palette: 'orange',
            }),
            top('Memory used of reserved', [sum('ecs.containerinsights.memory_utilized', 'servicename', s), sum('ecs.containerinsights.memory_reserved', 'servicename', s)], {
              formula: '100 * query1 / query2',
              palette: 'orange',
            }),
            ts('CPU by task definition family', avg('ecs.containerinsights.family.cpu_utilized', 'taskdefinitionfamily', fam)),
            ts('Memory by task definition family', avg('ecs.containerinsights.family.memory_utilized', 'taskdefinitionfamily', fam)),
          ]),
          group('Network, storage and restarts', [
            ts('Network received and sent', [sum('ecs.containerinsights.network_rx_bytes', null, s), sum('ecs.containerinsights.network_tx_bytes', null, s)], {
              formulas: [
                ['query1', 'received'],
                ['0 - query2', 'sent'],
              ],
              display: 'bars',
            }),
            ts('Storage read and written', [sum('ecs.containerinsights.storage_read_bytes', null, s), sum('ecs.containerinsights.storage_write_bytes', null, s)], { alias: ['read', 'written'] }),
            ts('Ephemeral storage used and reserved', [sum('ecs.containerinsights.ephemeral_storage_utilized', null, s), sum('ecs.containerinsights.ephemeral_storage_reserved', null, s)], {
              alias: ['used', 'reserved'],
            }),
            ts('Restarts and unhealthy containers', [sum('ecs.containerinsights.restart_count', null, s), sum('ecs.containerinsights.unhealthy_container_health_status', null, s)], {
              alias: ['restarts', 'unhealthy'],
              palette: 'warm',
            }),
          ]),
          group('Container instances', [
            ts('Instance CPU by cluster', avg('ecs.containerinsights.instance_cpu_utilization', 'clustername', c), { w: 4 }),
            ts('Instance memory by cluster', avg('ecs.containerinsights.instance_memory_utilization', 'clustername', c), { w: 4 }),
            ts('Instance filesystem by cluster', avg('ecs.containerinsights.instance_filesystem_utilization', 'clustername', c), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'AWS/NATGateway': {
    vars: ['natgatewayid'],
    monitor: 'amazon_nat_gateway',
    note: 'NAT gateways: traffic between your private subnets and the internet in both directions, connections and idle timeouts, and the port allocation errors and dropped packets that point to a gateway at its limits.',
    units: {
      BytesInFromDestination: 'byte',
      BytesInFromSource: 'byte',
      BytesOutToDestination: 'byte',
      BytesOutToSource: 'byte',
      PeakBytesPerSecond: 'byte',
      PacketsInFromDestination: 'packet',
      PacketsInFromSource: 'packet',
      PacketsOutToDestination: 'packet',
      PacketsOutToSource: 'packet',
      PacketsDropCount: 'packet',
      ActiveConnectionCount: 'connection',
    },
    build: ({ sum, max }) => ({
      kpis: [
        qv('Active connections', max('aws.natgateway.active_connection_count'), { agg: 'avg' }),
        qv('Sent to the internet', sum('aws.natgateway.bytes_out_to_destination')),
        qv('Received from the internet', sum('aws.natgateway.bytes_in_from_destination')),
        qv('Port allocation errors', sum('aws.natgateway.error_port_allocation'), { bad: true }),
        qv('Packets dropped', sum('aws.natgateway.packets_drop_count'), { bad: true }),
        qv('Idle timeouts', sum('aws.natgateway.idle_timeout_count')),
      ],
      groups: [
        group('Traffic', [
          ts('To and from the internet', [sum('aws.natgateway.bytes_out_to_destination'), sum('aws.natgateway.bytes_in_from_destination')], {
            formulas: [
              ['query1', 'sent'],
              ['0 - query2', 'received'],
            ],
            display: 'bars',
          }),
          ts('From and to your VPC', [sum('aws.natgateway.bytes_in_from_source'), sum('aws.natgateway.bytes_out_to_source')], {
            formulas: [
              ['query1', 'from VPC'],
              ['0 - query2', 'to VPC'],
            ],
            display: 'bars',
          }),
          top('Busiest gateways', sum('aws.natgateway.bytes_out_to_destination', 'natgatewayid')),
          ts('Peak bytes per second', max('aws.natgateway.peak_bytes_per_second', 'natgatewayid')),
        ]),
        group('Connections', [
          ts('Active connections by gateway', max('aws.natgateway.active_connection_count', 'natgatewayid')),
          ts('Attempted and established', [sum('aws.natgateway.connection_attempt_count'), sum('aws.natgateway.connection_established_count')], { alias: ['attempted', 'established'] }),
          ts('Idle timeouts by gateway', sum('aws.natgateway.idle_timeout_count', 'natgatewayid'), { display: 'bars', w: 12 }),
        ]),
        group('Errors and drops', [
          ts('Port allocation errors by gateway', sum('aws.natgateway.error_port_allocation', 'natgatewayid'), { display: 'bars', palette: 'warm' }),
          ts('Packets dropped by gateway', sum('aws.natgateway.packets_drop_count', 'natgatewayid'), { display: 'bars', palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/Route53': {
    vars: ['healthcheckid'],
    monitor: 'amazon_route53',
    note: 'Health checks: whether each endpoint is healthy and what share of Route 53’s checkers agree, and how long connecting, the TLS handshake and the first byte take.',
    logs: [['DNS query logs', 'source:route53']],
    units: { ConnectionTime: 'millisecond', SSLHandshakeTime: 'millisecond', TimeToFirstByte: 'millisecond', HealthCheckPercentageHealthy: 'percent' },
    build: ({ avg, min }) => ({
      kpis: [
        qv('Health checks', min('aws.route53.health_check_status', 'healthcheckid'), { formula: 'count_not_null(query1)' }),
        qv('Lowest status (1 healthy)', min('aws.route53.health_check_status'), { agg: 'min' }),
        qv('Checkers reporting healthy', avg('aws.route53.health_check_percentage_healthy'), { precision: 1 }),
        qv('Connection time', avg('aws.route53.connection_time')),
        qv('TLS handshake', avg('aws.route53.sslhandshake_time')),
        qv('Time to first byte', avg('aws.route53.time_to_first_byte')),
      ],
      groups: [
        group('Health', [
          ts('Status by health check (1 healthy, 0 unhealthy)', min('aws.route53.health_check_status', 'healthcheckid')),
          top('Least healthy checks', avg('aws.route53.health_check_percentage_healthy', 'healthcheckid'), { order: 'asc' }),
          ts('Checkers reporting healthy (%)', avg('aws.route53.health_check_percentage_healthy', 'healthcheckid')),
          ts('Healthy child checks (calculated checks)', avg('aws.route53.child_health_check_healthy_count', 'healthcheckid')),
        ]),
        group('Connection', [
          ts('Connection time by health check', avg('aws.route53.connection_time', 'healthcheckid'), { w: 8 }),
          top('Slowest to connect', avg('aws.route53.connection_time', 'healthcheckid'), { w: 4, palette: 'orange' }),
        ]),
        group('TLS handshake', [
          ts('TLS handshake by health check', avg('aws.route53.sslhandshake_time', 'healthcheckid'), { w: 8 }),
          top('Slowest handshakes', avg('aws.route53.sslhandshake_time', 'healthcheckid'), { w: 4, palette: 'orange' }),
        ]),
        group('First byte', [
          ts('Time to first byte by health check', avg('aws.route53.time_to_first_byte', 'healthcheckid'), { w: 8 }),
          top('Slowest first byte', avg('aws.route53.time_to_first_byte', 'healthcheckid'), { w: 4, palette: 'orange' }),
        ]),
      ],
    }),
  },

  'AWS/WAFV2': {
    vars: ['webacl'],
    monitor: 'amazon_waf',
    note: 'Web ACLs: requests allowed, blocked and counted (the ALL rule is each web ACL’s total), the rules and rule groups that matched, bot control’s CAPTCHAs and challenges, and DDoS attack requests.',
    logs: [['WAF logs', 'source:waf']],
    build: ({ sum }) => {
      const all = ['rule:ALL', 'webacl:*'];
      const rules = ['!rule:ALL', 'rule:*'];
      return {
        kpis: [
          qv('Allowed', sum('aws.wafv2.allowed_requests', null, 'rule:ALL'), { good: true }),
          qv('Blocked', sum('aws.wafv2.blocked_requests', null, 'rule:ALL'), { bad: true }),
          qv('Counted', sum('aws.wafv2.counted_requests', null, 'rule:ALL')),
          qv('Challenged', sum('aws.wafv2.challenge_requests', null, 'rule:ALL')),
          qv('CAPTCHA', sum('aws.wafv2.captcha_requests', null, 'rule:ALL')),
          qv('DDoS attack requests', sum('aws.wafv2.ddos_attack_requests'), { bad: true }),
        ],
        groups: [
          group('Requests', [
            ts('Allowed and blocked', [sum('aws.wafv2.allowed_requests', null, ...all), sum('aws.wafv2.blocked_requests', null, ...all)], { alias: ['allowed', 'blocked'], display: 'bars', w: 8 }),
            top('Most blocked web ACLs', sum('aws.wafv2.blocked_requests', 'webacl', ...all), { w: 4, palette: 'warm' }),
            ts('Blocked by web ACL', sum('aws.wafv2.blocked_requests', 'webacl', ...all), { palette: 'warm' }),
            ts('Counted by web ACL', sum('aws.wafv2.counted_requests', 'webacl', ...all)),
          ]),
          group('Rules', [
            top('Rules blocking the most', sum('aws.wafv2.blocked_requests', 'rule', ...rules), { palette: 'warm' }),
            top('Rules counting the most', sum('aws.wafv2.counted_requests', 'rule', ...rules)),
            ts('Requests passed by rule groups', sum('aws.wafv2.passed_requests', 'rulegroup', 'rulegroup:*')),
            ts('Labels matched by blocking rules', sum('aws.wafv2.block_rule_match', 'label', 'label:*'), { palette: 'warm' }),
          ]),
          group('CAPTCHAs and challenges', [
            ts('CAPTCHAs attempted and solved', [sum('aws.wafv2.captchas_attempted', null, 'rule:ALL'), sum('aws.wafv2.captchas_solved', null, 'rule:ALL')], { alias: ['attempted', 'solved'] }),
            ts('Challenges attempted and solved', [sum('aws.wafv2.challenges_attempted', null, 'rule:ALL'), sum('aws.wafv2.challenges_solved', null, 'rule:ALL')], { alias: ['attempted', 'solved'] }),
            ts('Requests with a valid token', [sum('aws.wafv2.requests_with_valid_captcha_token', null, 'rule:ALL'), sum('aws.wafv2.requests_with_valid_challenge_token', null, 'rule:ALL')], {
              alias: ['CAPTCHA', 'challenge'],
            }),
            ts('DDoS attack requests by web ACL', sum('aws.wafv2.ddos_attack_requests', 'webacl', 'webacl:*'), { display: 'bars', palette: 'warm' }),
          ]),
        ],
      };
    },
  },

  'AWS/Kinesis': {
    vars: ['streamname'],
    monitor: 'amazon_kinesis',
    note: 'Data streams: what producers put, what consumers read and how far behind they are (iterator age), and the reads and writes throttled for exceeding a shard’s throughput.',
    units: {
      IncomingBytes: 'byte',
      'GetRecords.Bytes': 'byte',
      'PutRecords.Bytes': 'byte',
      'GetRecords.IteratorAgeMilliseconds': 'millisecond',
      'GetRecords.Latency': 'millisecond',
      IncomingRecords: 'record',
      'GetRecords.Records': 'record',
    },
    build: ({ sum, max }) => ({
      kpis: [
        qv('Records in', sum('aws.kinesis.incoming_records')),
        qv('Bytes in', sum('aws.kinesis.incoming_bytes')),
        qv('Records read', sum('aws.kinesis.get_records_records')),
        qv('Furthest behind (ms)', max('aws.kinesis.get_records_iterator_age_milliseconds')),
        qv('Read throttles', sum('aws.kinesis.read_provisioned_throughput_exceeded'), { bad: true }),
        qv('Write throttles', sum('aws.kinesis.write_provisioned_throughput_exceeded'), { bad: true }),
      ],
      groups: [
        group('Producers', [
          ts('Records in by stream', sum('aws.kinesis.incoming_records', 'streamname'), { display: 'bars' }),
          ts('Bytes in by stream', sum('aws.kinesis.incoming_bytes', 'streamname')),
          ts('Successful puts', [sum('aws.kinesis.put_record_success'), sum('aws.kinesis.put_records_success')], { alias: ['PutRecord', 'PutRecords'], w: 12 }),
        ]),
        group('Consumers', [
          ts('Iterator age by stream', max('aws.kinesis.get_records_iterator_age_milliseconds', 'streamname')),
          top('Furthest behind', max('aws.kinesis.get_records_iterator_age_milliseconds', 'streamname'), { palette: 'orange' }),
          ts('Records read by stream', sum('aws.kinesis.get_records_records', 'streamname'), { w: 4 }),
          ts('Bytes read by stream', sum('aws.kinesis.get_records_bytes', 'streamname'), { w: 4 }),
          ts('GetRecords latency', max('aws.kinesis.get_records_latency', 'streamname'), { w: 4 }),
        ]),
        group('Throttling', [
          ts('Reads throttled by stream', sum('aws.kinesis.read_provisioned_throughput_exceeded', 'streamname'), { display: 'bars', palette: 'warm' }),
          ts('Writes throttled by stream', sum('aws.kinesis.write_provisioned_throughput_exceeded', 'streamname'), { display: 'bars', palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/Firehose': {
    vars: ['deliverystreamname'],
    monitor: 'amazon_firehose',
    note: 'Delivery streams: records and bytes coming in, what reaches each destination (S3, OpenSearch, Redshift, Splunk) and how fresh it is when it gets there.',
    units: {
      IncomingBytes: 'byte',
      'DeliveryToS3.Bytes': 'byte',
      'DeliveryToS3.DataFreshness': 'second',
      'DeliveryToSplunk.DataFreshness': 'second',
      'DeliveryToSplunk.DataAckLatency': 'second',
      'BackupToS3.DataFreshness': 'second',
      KinesisMillisBehindLatest: 'millisecond',
      IncomingRecords: 'record',
      'DeliveryToS3.Records': 'record',
    },
    build: ({ sum, max }) => ({
      kpis: [
        qv('Records in', sum('aws.firehose.incoming_records')),
        qv('Bytes in', sum('aws.firehose.incoming_bytes')),
        qv('Records delivered to S3', sum('aws.firehose.delivery_to_s3_records')),
        qv('Oldest record in S3 delivery (s)', max('aws.firehose.delivery_to_s3_data_freshness.maximum')),
        qv('Behind the source stream (ms)', max('aws.firehose.kinesis_millis_behind_latest.maximum')),
      ],
      groups: [
        group('Ingestion', [
          ts('Records in by stream', sum('aws.firehose.incoming_records', 'deliverystreamname'), { display: 'bars' }),
          ts('Bytes in by stream', sum('aws.firehose.incoming_bytes', 'deliverystreamname')),
          ts('Records read from Kinesis Data Streams', sum('aws.firehose.data_read_from_kinesis_stream_records', 'deliverystreamname')),
          ts('Behind the source stream', max('aws.firehose.kinesis_millis_behind_latest.maximum', 'deliverystreamname')),
        ]),
        group('Delivery to S3', [
          ts('Records delivered by stream', sum('aws.firehose.delivery_to_s3_records', 'deliverystreamname'), { display: 'bars' }),
          ts('Bytes delivered by stream', sum('aws.firehose.delivery_to_s3_bytes', 'deliverystreamname')),
          ts('Data freshness by stream', max('aws.firehose.delivery_to_s3_data_freshness.maximum', 'deliverystreamname')),
          ts('Successful deliveries by stream', sum('aws.firehose.delivery_to_s3_success', 'deliverystreamname')),
        ]),
        group('Other destinations', [
          ts(
            'Records delivered by destination',
            [sum('aws.firehose.delivery_to_elasticsearch_records'), sum('aws.firehose.delivery_to_redshift_records'), sum('aws.firehose.delivery_to_splunk_records')],
            { alias: ['OpenSearch', 'Redshift', 'Splunk'] },
          ),
          ts('Splunk acknowledgement latency', max('aws.firehose.delivery_to_splunk_data_ack_latency.maximum', 'deliverystreamname')),
          ts('Splunk data freshness', max('aws.firehose.delivery_to_splunk_data_freshness.maximum', 'deliverystreamname')),
          ts('Records backed up to S3', sum('aws.firehose.backup_to_s3_records', 'deliverystreamname')),
        ]),
      ],
    }),
  },

  'AWS/States': {
    vars: ['statemachinearn', 'activityarn'],
    monitor: 'amazon_step_functions',
    note: 'State machines: executions and how they ended, how long they take and whether they are throttled, and the activities workers poll for.',
    logs: [['Step Functions logs', 'source:stepfunction']],
    units: { ExecutionTime: 'millisecond', ActivityRunTime: 'millisecond', ActivityScheduleTime: 'millisecond', ActivityTime: 'millisecond' },
    build: ({ sum, avg }) => ({
      kpis: [
        qv('Executions started', sum('aws.states.executions_started')),
        qv('Succeeded', sum('aws.states.executions_succeeded'), { good: true }),
        qv('Failed', sum('aws.states.executions_failed'), { bad: true }),
        qv('Timed out', sum('aws.states.executions_timed_out'), { bad: true }),
        qv('Aborted', sum('aws.states.executions_aborted')),
        qv('Avg execution time', avg('aws.states.execution_time')),
      ],
      groups: [
        group('Executions', [
          ts(
            'How executions ended',
            ['succeeded', 'failed', 'timed_out', 'aborted'].map((o) => sum(`aws.states.executions_${o}`)),
            { alias: ['succeeded', 'failed', 'timed out', 'aborted'], display: 'bars', w: 8 },
          ),
          top('Most started', sum('aws.states.executions_started', 'statemachinearn'), { w: 4 }),
          ts('Started by state machine', sum('aws.states.executions_started', 'statemachinearn')),
          ts('Failure rate by state machine', [sum('aws.states.executions_failed', 'statemachinearn'), sum('aws.states.executions_started', 'statemachinearn')], {
            formulas: [['100 * query1 / query2', 'failed %']],
            palette: 'warm',
          }),
        ]),
        group('Execution time', [
          ts('Execution time by state machine', avg('aws.states.execution_time', 'statemachinearn')),
          top('Slowest state machines', avg('aws.states.execution_time', 'statemachinearn'), { palette: 'orange' }),
          heat('Execution time spread', avg('aws.states.execution_time', 'statemachinearn')),
          ts('Throttled executions', sum('aws.states.execution_throttled', 'statemachinearn'), { display: 'bars', palette: 'warm' }),
        ]),
        group('Activities', [
          ts(
            'How activities ended',
            ['succeeded', 'failed', 'timed_out', 'heartbeat_timed_out'].map((o) => sum(`aws.states.activities_${o}`)),
            { alias: ['succeeded', 'failed', 'timed out', 'heartbeat timed out'], display: 'bars' },
          ),
          ts('Waiting and running time', [avg('aws.states.activity_schedule_time'), avg('aws.states.activity_run_time')], { alias: ['waiting for a worker', 'running'] }),
          top('Slowest activities', avg('aws.states.activity_time', 'activityarn'), { palette: 'orange' }),
          ts('Activities scheduled', sum('aws.states.activities_scheduled', 'activityarn'), { display: 'bars' }),
        ]),
      ],
    }),
  },

  'AWS/EFS': {
    vars: ['filesystemid'],
    monitor: 'amazon_efs',
    note: 'File systems: data and metadata I/O, the throughput used against what is permitted, the share of the I/O limit used (General Purpose mode), client connections, storage by class and burst credits.',
    units: {
      DataReadIOBytes: 'byte',
      DataWriteIOBytes: 'byte',
      MetadataIOBytes: 'byte',
      MeteredIOBytes: 'byte',
      TotalIOBytes: 'byte',
      StorageBytes: 'byte',
      BurstCreditBalance: 'byte',
      PermittedThroughput: 'byte/second',
      PercentIOLimit: 'percent',
    },
    build: ({ sum, avg, max, min }) => ({
      kpis: [
        qv('Client connections', sum('aws.efs.client_connections'), { agg: 'avg' }),
        qv('Total I/O', sum('aws.efs.total_iobytes')),
        qv('Highest I/O limit used', max('aws.efs.percent_iolimit.maximum'), { precision: 1 }),
        qv('Storage', sum('aws.efs.storage_bytes', null, 'storageclass:total'), { agg: 'last' }),
        qv('Lowest burst credits', min('aws.efs.burst_credit_balance')),
      ],
      groups: [
        group('Throughput', [
          ts('Data and metadata I/O', [sum('aws.efs.data_read_iobytes.sum'), sum('aws.efs.data_write_iobytes.sum'), sum('aws.efs.metadata_iobytes.sum')], {
            alias: ['read', 'write', 'metadata'],
            display: 'bars',
          }),
          ts('Throughput used and permitted', [sum('aws.efs.metered_iobytes.sum'), sum('aws.efs.permitted_throughput')], {
            formulas: [
              ['query1 / 60', 'used'],
              ['query2', 'permitted'],
            ],
          }),
          top('Busiest file systems', sum('aws.efs.total_iobytes', 'filesystemid')),
          ts('I/O limit used by file system', max('aws.efs.percent_iolimit.maximum', 'filesystemid')),
        ]),
        group('Connections', [
          ts('Client connections by file system', sum('aws.efs.client_connections', 'filesystemid'), { w: 8 }),
          top('Most connected', sum('aws.efs.client_connections', 'filesystemid'), { w: 4 }),
        ]),
        group('Storage and burst credits', [
          ts('Storage by class', avg('aws.efs.storage_bytes', 'storageclass'), { display: 'area' }),
          top('Largest file systems', avg('aws.efs.storage_bytes', 'filesystemid', 'storageclass:total')),
          ts('Burst credits by file system', avg('aws.efs.burst_credit_balance', 'filesystemid')),
          top('Fewest burst credits', min('aws.efs.burst_credit_balance', 'filesystemid'), { order: 'asc' }),
        ]),
      ],
    }),
  },

  'AWS/ES': {
    vars: ['domainname'],
    monitor: 'amazon_es',
    note: 'OpenSearch Service domains: cluster status and shards, free storage and documents, CPU and JVM memory pressure of data and dedicated master nodes, and search and indexing performance.',
    units: {
      CPUUtilization: 'percent',
      JVMMemoryPressure: 'percent',
      MasterCPUUtilization: 'percent',
      MasterJVMMemoryPressure: 'percent',
      FreeStorageSpace: 'megabyte',
      ClusterUsedSpace: 'megabyte',
      SearchLatency: 'millisecond',
      IndexingLatency: 'millisecond',
      ReadLatency: 'second',
      WriteLatency: 'second',
    },
    build: ({ sum, avg, max, min }) => ({
      kpis: [
        qv('Red domains', max('aws.es.cluster_status_red', 'domainname'), { formula: 'count_nonzero(query1)', bad: true }),
        qv('Yellow domains', max('aws.es.cluster_status_yellow', 'domainname'), { formula: 'count_nonzero(query1)' }),
        qv('Nodes', sum('aws.es.nodes'), { agg: 'last' }),
        qv('Lowest free storage', min('aws.es.free_storage_space'), { agg: 'min' }),
        qv('Highest JVM pressure', max('aws.es.jvmmemory_pressure'), { precision: 1 }),
        qv('Index writes blocked', max('aws.es.cluster_index_writes_blocked'), { bad: true }),
      ],
      groups: [
        group('Cluster health', [
          ts('Cluster status', [max('aws.es.cluster_status_green'), max('aws.es.cluster_status_yellow'), max('aws.es.cluster_status_red')], { alias: ['green', 'yellow', 'red'] }),
          top('Domains in red', max('aws.es.cluster_status_red', 'domainname'), { palette: 'warm' }),
          ts(
            'Shards',
            ['active', 'unassigned', 'initializing', 'relocating'].map((s) => sum(`aws.es.shards_${s}`)),
            { alias: ['active', 'unassigned', 'initializing', 'relocating'] },
          ),
          ts('Nodes by domain', max('aws.es.nodes', 'domainname')),
        ]),
        group('Storage and documents', [
          ts('Free storage by domain', min('aws.es.free_storage_space', 'domainname')),
          ts('Used space by domain', max('aws.es.cluster_used_space', 'domainname')),
          ts('Searchable and deleted documents', [avg('aws.es.searchable_documents'), avg('aws.es.deleted_documents')], { alias: ['searchable', 'deleted'] }),
          ts('Automated snapshot failures', max('aws.es.automated_snapshot_failure', 'domainname'), { palette: 'warm' }),
        ]),
        group('Node health', [
          ts('Highest CPU by domain', max('aws.es.cpuutilization.maximum', 'domainname')),
          ts('JVM memory pressure by domain', max('aws.es.jvmmemory_pressure', 'domainname')),
          ts('Master nodes: CPU and JVM pressure', [max('aws.es.master_cpuutilization'), max('aws.es.master_jvmmemory_pressure')], { alias: ['CPU', 'JVM pressure'] }),
          ts('Rejected thread pool tasks', [max('aws.es.threadpool_search_rejected'), max('aws.es.threadpool_write_rejected'), max('aws.es.threadpool_bulk_rejected')], {
            alias: ['search', 'write', 'bulk'],
            palette: 'warm',
          }),
        ]),
        group('Performance', [
          ts('Search and indexing rate', [avg('aws.es.search_rate'), avg('aws.es.indexing_rate')], { alias: ['search', 'indexing'] }),
          ts('Search and indexing latency', [avg('aws.es.search_latency'), avg('aws.es.indexing_latency')], { alias: ['search', 'indexing'] }),
          ts('Disk latency', [avg('aws.es.read_latency'), avg('aws.es.write_latency')], { alias: ['read', 'write'] }),
          ts('Disk IOPS', [avg('aws.es.read_iops'), avg('aws.es.write_iops')], { alias: ['read', 'write'] }),
          ts(
            'Requests by response class',
            ['2xx', '3xx', '4xx', '5xx'].map((c) => sum(`aws.es.${c}`)),
            { alias: ['2xx', '3xx', '4xx', '5xx'], display: 'bars' },
          ),
          ts('Throughput and IOPS throttling', [max('aws.es.throughput_throttle'), max('aws.es.iops_throttle')], { alias: ['throughput', 'IOPS'], palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/Redshift': {
    vars: ['clusteridentifier'],
    monitor: 'amazon_redshift',
    note: 'Provisioned clusters: health, CPU and disk space, queries by duration band, workload management queues and concurrency scaling, and storage I/O.',
    logs: [['Redshift logs', 'source:redshift']],
    units: { CPUUtilization: 'percent', PercentageDiskSpaceUsed: 'percent', ReadLatency: 'second', WriteLatency: 'second' },
    build: ({ sum, avg, max, min }) => ({
      kpis: [
        qv('Health (1 healthy)', min('aws.redshift.health_status'), { agg: 'min' }),
        qv('Avg CPU', avg('aws.redshift.cpuutilization'), { precision: 1 }),
        qv('Most disk used', max('aws.redshift.percentage_disk_space_used'), { precision: 1 }),
        qv('Connections', sum('aws.redshift.database_connections'), { agg: 'avg' }),
        qv('Queries per second', sum('aws.redshift.queries_completed_per_second'), { agg: 'avg', precision: 1 }),
        qv('Longest WLM queue', max('aws.redshift.wlmqueue_length')),
      ],
      groups: [
        group('Cluster health', [
          ts('Health by cluster', min('aws.redshift.health_status', 'clusteridentifier'), { w: 4 }),
          ts('In maintenance', max('aws.redshift.maintenance_mode', 'clusteridentifier'), { w: 4 }),
          ts('CPU by cluster', avg('aws.redshift.cpuutilization', 'clusteridentifier'), { w: 4 }),
          top('Fullest clusters', max('aws.redshift.percentage_disk_space_used', 'clusteridentifier'), { palette: 'orange' }),
          ts('Disk space used by cluster', avg('aws.redshift.percentage_disk_space_used', 'clusteridentifier')),
        ]),
        group('Queries', [
          ts('Queries completed by duration band', avg('aws.redshift.queries_completed_per_second', 'latency')),
          ts('Query duration by band', avg('aws.redshift.query_duration', 'latency')),
          ts('Query runtime by stage', avg('aws.redshift.query_runtime_breakdown', 'stage'), { display: 'area' }),
          ts('Connections by cluster', avg('aws.redshift.database_connections', 'clusteridentifier')),
        ]),
        group('Workload management', [
          ts('Queue length by cluster', avg('aws.redshift.wlmqueue_length', 'clusteridentifier')),
          ts('Queue wait time by queue', avg('aws.redshift.wlmqueue_wait_time', 'wlmid')),
          ts('Running queries by queue', avg('aws.redshift.wlmrunning_queries', 'wlmid')),
          ts('Concurrency scaling clusters', avg('aws.redshift.concurrency_scaling_active_clusters', 'clusteridentifier')),
        ]),
        group('Storage I/O', [
          ts('Read and write IOPS', [sum('aws.redshift.read_iops'), sum('aws.redshift.write_iops')], { alias: ['read', 'write'] }),
          ts('Read and write latency', [avg('aws.redshift.read_latency'), avg('aws.redshift.write_latency')], { alias: ['read', 'write'] }),
          ts('Read and write throughput', [sum('aws.redshift.read_throughput'), sum('aws.redshift.write_throughput')], { alias: ['read', 'write'] }),
          ts('Network throughput', [sum('aws.redshift.network_receive_throughput'), sum('aws.redshift.network_transmit_throughput')], {
            formulas: [
              ['query1', 'received'],
              ['0 - query2', 'sent'],
            ],
          }),
        ]),
      ],
    }),
  },

  'AWS/Kafka': {
    vars: ['cluster_name', 'broker_id'],
    monitor: 'amazon_msk',
    note: 'MSK clusters: bytes and messages in and out per broker, under-replicated partitions, broker CPU, disk and memory, and client connections. Topic and consumer-group lag come from the Agent’s Kafka checks.',
    units: {
      BytesInPerSec: 'byte/second',
      BytesOutPerSec: 'byte/second',
      KafkaDataLogsDiskUsed: 'percent',
      KafkaAppLogsDiskUsed: 'percent',
      RootDiskUsed: 'percent',
      CpuUser: 'percent',
      CpuSystem: 'percent',
      CpuIoWait: 'percent',
      HeapMemoryAfterGC: 'percent',
      MemoryUsed: 'byte',
      MemoryFree: 'byte',
      ProduceTotalTimeMsMean: 'millisecond',
    },
    build: ({ sum, avg, max }) => ({
      kpis: [
        qv('Bytes in per second', sum('aws.kafka.bytes_in_per_sec'), { agg: 'avg' }),
        qv('Bytes out per second', sum('aws.kafka.bytes_out_per_sec'), { agg: 'avg' }),
        qv('Messages in per second', sum('aws.kafka.messages_in_per_sec'), { agg: 'avg' }),
        qv('Under-replicated partitions', sum('aws.kafka.under_replicated_partitions'), { agg: 'max', bad: true }),
        qv('Most data disk used', max('aws.kafka.kafka_data_logs_disk_used'), { precision: 1 }),
        qv('Busiest broker CPU', [max('aws.kafka.cpu_user'), max('aws.kafka.cpu_system')], { formula: 'query1 + query2', precision: 1 }),
      ],
      groups: [
        group('Throughput', [
          ts('Bytes in by broker', sum('aws.kafka.bytes_in_per_sec', 'broker_id')),
          ts('Bytes out by broker', sum('aws.kafka.bytes_out_per_sec', 'broker_id')),
          ts('Messages in by cluster', sum('aws.kafka.messages_in_per_sec', 'cluster_name')),
          ts('Produce time (mean)', avg('aws.kafka.produce_total_time_ms_mean', 'broker_id')),
        ]),
        group('Replication and partitions', [
          ts('Under-replicated partitions', sum('aws.kafka.under_replicated_partitions', 'cluster_name'), { palette: 'warm' }),
          ts('Partitions under minimum in-sync replicas', sum('aws.kafka.under_min_isr_partition_count', 'cluster_name'), { palette: 'warm' }),
          ts('Partitions by broker', sum('aws.kafka.partition_count', 'broker_id')),
          ts('Leaders by broker', sum('aws.kafka.leader_count', 'broker_id')),
        ]),
        group('Broker resources', [
          ts('CPU by broker (user + system)', [avg('aws.kafka.cpu_user', 'broker_id'), avg('aws.kafka.cpu_system', 'broker_id')], { formulas: [['query1 + query2', 'CPU']] }),
          ts('Data disk used by broker', max('aws.kafka.kafka_data_logs_disk_used', 'broker_id')),
          ts('Heap used after GC by broker', max('aws.kafka.heap_memory_after_gc', 'broker_id')),
          ts('Memory used and free', [avg('aws.kafka.memory_used'), avg('aws.kafka.memory_free')], { alias: ['used', 'free'] }),
        ]),
        group('Network and connections', [
          ts('Connections by broker', sum('aws.kafka.connection_count', 'broker_id')),
          ts('Client connections by authentication', sum('aws.kafka.client_connection_count', 'client_authentication')),
          ts(
            'Network errors and drops',
            ['rx_errors', 'tx_errors', 'rx_dropped', 'tx_dropped'].map((n) => sum(`aws.kafka.network_${n}`)),
            { alias: ['receive errors', 'send errors', 'receive drops', 'send drops'], palette: 'warm' },
          ),
          ts('Traffic shaping (throttled packets)', sum('aws.kafka.traffic_shaping', 'broker_id'), { palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/KMS': {
    vars: ['keyid'],
    monitor: 'amazon_kms',
    note: 'Keys with imported key material: how long until that material expires. Once it does, the key can no longer encrypt or decrypt until new material is imported.',
    units: { SecondsUntilKeyMaterialExpiration: 'second' },
    build: ({ min }) => ({
      kpis: [
        qv('Keys with expiring material', min('aws.kms.seconds_until_key_material_expiration', 'keyid'), { formula: 'count_not_null(query1)' }),
        qv('Soonest expiry (days)', min('aws.kms.seconds_until_key_material_expiration'), { formula: 'query1 / 86400', agg: 'min', precision: 1 }),
        qv('Latest expiry (days)', min('aws.kms.seconds_until_key_material_expiration.maximum'), { formula: 'query1 / 86400', agg: 'max', precision: 1 }),
      ],
      groups: [
        group('Expiry', [
          top('Expiring soonest (days)', min('aws.kms.seconds_until_key_material_expiration', 'keyid'), { formula: 'query1 / 86400', order: 'asc', w: 6, h: 3 }),
          ts('Days left by key', min('aws.kms.seconds_until_key_material_expiration', 'keyid'), { formulas: [['query1 / 86400', 'days left']], w: 6, h: 3 }),
        ]),
        group('Keys', [tbl('Keys with imported material', [['Days left', min('aws.kms.seconds_until_key_material_expiration', 'keyid'), 'min']])]),
      ],
    }),
  },

  'AWS/Bedrock': {
    vars: ['modelid'],
    monitor: 'amazon_bedrock',
    note: 'Foundation model calls: invocations and latency (and time to the first token when streaming) by model, client and server errors and throttles, tokens in and out, and the delivery of invocation logs.',
    units: {
      InvocationLatency: 'millisecond',
      TimeToFirstToken: 'millisecond',
      InputTokenCount: 'token',
      OutputTokenCount: 'token',
      CacheReadInputTokenCount: 'token',
      CacheWriteInputTokenCount: 'token',
    },
    build: ({ sum, avg }) => ({
      kpis: [
        qv('Invocations', sum('aws.bedrock.invocations')),
        qv('Client errors', sum('aws.bedrock.invocation_client_errors'), { bad: true }),
        qv('Server errors', sum('aws.bedrock.invocation_server_errors'), { bad: true }),
        qv('Throttles', sum('aws.bedrock.invocation_throttles'), { bad: true }),
        qv('Avg latency', avg('aws.bedrock.invocation_latency')),
        qv('Tokens', [sum('aws.bedrock.input_token_count'), sum('aws.bedrock.output_token_count')], { formula: 'query1 + query2' }),
      ],
      groups: [
        group('Invocations', [
          ts('Invocations by model', sum('aws.bedrock.invocations', 'modelid'), { display: 'bars', w: 8 }),
          top('Most used models', sum('aws.bedrock.invocations', 'modelid'), { w: 4 }),
          ts('Latency by model', avg('aws.bedrock.invocation_latency', 'modelid')),
          ts('Time to first token by model', avg('aws.bedrock.time_to_first_token', 'modelid')),
        ]),
        group('Errors and throttling', [
          ts('Errors and throttles', [sum('aws.bedrock.invocation_client_errors'), sum('aws.bedrock.invocation_server_errors'), sum('aws.bedrock.invocation_throttles')], {
            alias: ['client errors', 'server errors', 'throttles'],
            display: 'bars',
            palette: 'warm',
          }),
          ts('Error rate by model', [sum('aws.bedrock.invocation_client_errors', 'modelid'), sum('aws.bedrock.invocation_server_errors', 'modelid'), sum('aws.bedrock.invocations', 'modelid')], {
            formulas: [['100 * (query1 + query2) / query3', 'error %']],
            palette: 'warm',
          }),
          top('Most throttled models', sum('aws.bedrock.invocation_throttles', 'modelid'), { palette: 'warm' }),
          ts('Tokens-per-minute quota used (estimate)', avg('aws.bedrock.estimated_tpmquota_usage', 'modelid')),
        ]),
        group('Tokens', [
          ts('Input and output tokens', [sum('aws.bedrock.input_token_count'), sum('aws.bedrock.output_token_count')], { alias: ['input', 'output'], display: 'bars' }),
          top('Most tokens by model', [sum('aws.bedrock.input_token_count', 'modelid'), sum('aws.bedrock.output_token_count', 'modelid')], { formula: 'query1 + query2' }),
          ts('Prompt cache reads and writes', [sum('aws.bedrock.cache_read_input_token_count'), sum('aws.bedrock.cache_write_input_token_count')], { alias: ['read', 'written'] }),
          ts('Images generated by model', sum('aws.bedrock.output_image_count', 'modelid'), { display: 'bars' }),
        ]),
        group('Invocation logging', [
          ts(
            'Invocation log delivery',
            [
              sum('aws.bedrock.model_invocation_logs_cloud_watch_delivery_success'),
              sum('aws.bedrock.model_invocation_logs_cloud_watch_delivery_failure'),
              sum('aws.bedrock.model_invocation_logs_s3delivery_success'),
              sum('aws.bedrock.model_invocation_logs_s3delivery_failure'),
            ],
            { alias: ['CloudWatch delivered', 'CloudWatch failed', 'S3 delivered', 'S3 failed'], w: 12 },
          ),
        ]),
      ],
    }),
  },

  'AWS/AppRunner': {
    vars: ['servicename'],
    monitor: 'amazon_app_runner',
    note: 'App Runner services: requests and responses by class, latency and concurrency, and the instances running them with their CPU and memory.',
    units: { CPUUtilization: 'percent', MemoryUtilization: 'percent', RequestLatency: 'millisecond', Requests: 'request', ActiveInstances: 'instance' },
    build: ({ sum, avg }) => ({
      kpis: [
        qv('Requests', sum('aws.apprunner.requests')),
        qv('5xx responses', sum('aws.apprunner.5xx_status_responses'), { bad: true }),
        qv('Avg latency', avg('aws.apprunner.request_latency')),
        qv('Active instances', sum('aws.apprunner.active_instances'), { agg: 'last' }),
        qv('Avg CPU', avg('aws.apprunner.cpuutilization'), { precision: 1 }),
        qv('Avg memory', avg('aws.apprunner.memory_utilization'), { precision: 1 }),
      ],
      groups: [
        group('Traffic', [
          ts('Requests by service', sum('aws.apprunner.requests', 'servicename'), { display: 'bars' }),
          ts(
            'Responses by class',
            ['2xx', '4xx', '5xx'].map((c) => sum(`aws.apprunner.${c}_status_responses`)),
            { alias: ['2xx', '4xx', '5xx'], display: 'bars' },
          ),
          ts('Latency by service', avg('aws.apprunner.request_latency', 'servicename')),
          ts('Concurrent requests by service', avg('aws.apprunner.concurrency', 'servicename')),
        ]),
        group('Instances', [
          ts('Active instances by service', sum('aws.apprunner.active_instances', 'servicename')),
          top('Most CPU', avg('aws.apprunner.cpuutilization', 'servicename'), { palette: 'orange' }),
          ts('CPU by service', avg('aws.apprunner.cpuutilization', 'servicename')),
          ts('Memory by service', avg('aws.apprunner.memory_utilization', 'servicename')),
        ]),
        group('Errors', [
          ts('5xx rate by service', [sum('aws.apprunner.5xx_status_responses', 'servicename'), sum('aws.apprunner.requests', 'servicename')], {
            formulas: [['100 * query1 / query2', '5xx %']],
            palette: 'warm',
          }),
          top('Most 5xx', sum('aws.apprunner.5xx_status_responses', 'servicename'), { palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/Backup': {
    vars: ['backupvaultname'],
    monitor: 'amazon_backup',
    note: 'Backup vaults: backup, copy and restore jobs by outcome, and the recovery points they hold. A failed or expired job leaves a resource without a recent backup.',
    build: ({ sum, max }) => ({
      kpis: [
        qv('Backups completed', sum('aws.backup.number_of_backup_jobs_completed'), { good: true }),
        qv('Backups failed', sum('aws.backup.number_of_backup_jobs_failed'), { bad: true }),
        qv('Backups running', max('aws.backup.number_of_backup_jobs_running.maximum')),
        qv('Copies failed', sum('aws.backup.number_of_copy_jobs_failed'), { bad: true }),
        qv('Restores failed', sum('aws.backup.number_of_restore_jobs_failed'), { bad: true }),
        qv('Recovery points created', sum('aws.backup.number_of_recovery_points_completed')),
      ],
      groups: [
        group('Backup jobs', [
          ts(
            'Backup jobs by outcome',
            ['created', 'completed', 'failed', 'aborted', 'expired'].map((o) => sum(`aws.backup.number_of_backup_jobs_${o}`)),
            { alias: ['created', 'completed', 'failed', 'aborted', 'expired'], display: 'bars', w: 8 },
          ),
          top('Vaults with failed backups', sum('aws.backup.number_of_backup_jobs_failed', 'backupvaultname'), { w: 4, palette: 'warm' }),
          ts('Running and pending', [sum('aws.backup.number_of_backup_jobs_running'), sum('aws.backup.number_of_backup_jobs_pending')], { alias: ['running', 'pending'] }),
          ts('Failed backups by vault', sum('aws.backup.number_of_backup_jobs_failed', 'backupvaultname'), { display: 'bars', palette: 'warm' }),
        ]),
        group('Copy jobs', [
          ts(
            'Copy jobs by outcome',
            ['created', 'running', 'completed', 'failed'].map((o) => sum(`aws.backup.number_of_copy_jobs_${o}`)),
            { alias: ['created', 'running', 'completed', 'failed'] },
          ),
          ts('Failed copies by vault', sum('aws.backup.number_of_copy_jobs_failed', 'backupvaultname'), { display: 'bars', palette: 'warm' }),
        ]),
        group('Restore jobs', [
          ts(
            'Restore jobs by outcome',
            ['pending', 'running', 'completed', 'failed'].map((o) => sum(`aws.backup.number_of_restore_jobs_${o}`)),
            { alias: ['pending', 'running', 'completed', 'failed'] },
          ),
          top('Vaults with failed restores', sum('aws.backup.number_of_restore_jobs_failed', 'backupvaultname'), { palette: 'warm' }),
        ]),
        group('Recovery points', [
          ts(
            'Recovery points by state',
            ['completed', 'partial', 'expired', 'deleting', 'cold'].map((o) => sum(`aws.backup.number_of_recovery_points_${o}`)),
            { alias: ['completed', 'partial', 'expired', 'deleting', 'cold storage'], display: 'bars' },
          ),
          top('Vaults by recovery points created', sum('aws.backup.number_of_recovery_points_completed', 'backupvaultname')),
        ]),
      ],
    }),
  },

  'AWS/CertificateManager': {
    vars: ['certificatearn'],
    monitor: 'amazon_certificate_manager',
    note: 'Certificates: the days left before each one expires (renewal is automatic only for DNS-validated certificates in use), and certificates issued through ACME endpoints.',
    units: { DaysToExpiry: 'day' },
    build: ({ sum, min }) => ({
      kpis: [
        qv('Certificates', min('aws.certificatemanager.days_to_expiry', 'certificatearn'), { formula: 'count_not_null(query1)' }),
        qv('Soonest expiry (days)', min('aws.certificatemanager.days_to_expiry'), { agg: 'min' }),
        qv('ACME certificates issued', sum('aws.certificatemanager.certificate_issuance_success'), { good: true }),
        qv('ACME issuance failures', sum('aws.certificatemanager.certificate_issuance_failed'), { bad: true }),
      ],
      groups: [
        group('Expiry', [
          top('Expiring soonest', min('aws.certificatemanager.days_to_expiry', 'certificatearn'), { order: 'asc', w: 6, h: 3 }),
          ts('Days to expiry by certificate', min('aws.certificatemanager.days_to_expiry', 'certificatearn'), { w: 6, h: 3 }),
        ]),
        group('ACME issuance', [
          ts('Certificates issued and failed', [sum('aws.certificatemanager.certificate_issuance_success'), sum('aws.certificatemanager.certificate_issuance_failed')], {
            alias: ['issued', 'failed'],
            display: 'bars',
          }),
          top('Endpoints with most failures', sum('aws.certificatemanager.certificate_issuance_failed', 'acmeendpointarn'), { palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/PrivateLinkEndpoints': {
    vars: ['vpc_endpoint_id', 'service_name', 'vpc_id'],
    monitor: 'amazon_privatelink',
    note: 'Interface and Gateway Load Balancer endpoints: connections and bytes to the services behind them, and the packets dropped or reset on the way.',
    units: { BytesProcessed: 'byte', ActiveConnections: 'connection', NewConnections: 'connection', PacketsDropped: 'packet', RstPacketsReceived: 'packet' },
    build: ({ sum }) => ({
      kpis: [
        qv('Active connections', sum('aws.privatelinkendpoints.active_connections'), { agg: 'avg' }),
        qv('New connections', sum('aws.privatelinkendpoints.new_connections')),
        qv('Bytes processed', sum('aws.privatelinkendpoints.bytes_processed')),
        qv('Packets dropped', sum('aws.privatelinkendpoints.packets_dropped'), { bad: true }),
        qv('Resets received', sum('aws.privatelinkendpoints.rst_packets_received'), { bad: true }),
      ],
      groups: [
        group('Connections', [
          ts('Active connections by endpoint', sum('aws.privatelinkendpoints.active_connections', 'vpc_endpoint_id')),
          ts('New connections by endpoint', sum('aws.privatelinkendpoints.new_connections', 'vpc_endpoint_id'), { display: 'bars' }),
        ]),
        group('Traffic', [
          ts('Bytes by endpoint', sum('aws.privatelinkendpoints.bytes_processed', 'vpc_endpoint_id')),
          top('Busiest services', sum('aws.privatelinkendpoints.bytes_processed', 'service_name')),
        ]),
        group('Drops and resets', [
          ts('Packets dropped by endpoint', sum('aws.privatelinkendpoints.packets_dropped', 'vpc_endpoint_id'), { display: 'bars', palette: 'warm' }),
          ts('Resets received by endpoint', sum('aws.privatelinkendpoints.rst_packets_received', 'vpc_endpoint_id'), { display: 'bars', palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/PrivateLinkServices': {
    vars: ['service_id'],
    monitor: 'amazon_privatelink',
    note: 'Endpoint services you provide: how many endpoints connect to each, their connections and bytes, and the resets your service sends back.',
    units: { BytesProcessed: 'byte', ActiveConnections: 'connection', NewConnections: 'connection', RstPacketsSent: 'packet', EndpointsCount: 'endpoint' },
    build: ({ sum }) => ({
      kpis: [
        qv('Connected endpoints', sum('aws.privatelinkservices.endpoints_count'), { agg: 'last' }),
        qv('Active connections', sum('aws.privatelinkservices.active_connections'), { agg: 'avg' }),
        qv('New connections', sum('aws.privatelinkservices.new_connections')),
        qv('Bytes processed', sum('aws.privatelinkservices.bytes_processed')),
        qv('Resets sent', sum('aws.privatelinkservices.rst_packets_sent'), { bad: true }),
      ],
      groups: [
        group('Consumers', [
          ts('Endpoints by service', sum('aws.privatelinkservices.endpoints_count', 'service_id')),
          top('Most connected services', sum('aws.privatelinkservices.endpoints_count', 'service_id'), { agg: 'last' }),
        ]),
        group('Connections', [
          ts('Active connections by service', sum('aws.privatelinkservices.active_connections', 'service_id')),
          ts('New connections by service', sum('aws.privatelinkservices.new_connections', 'service_id'), { display: 'bars' }),
        ]),
        group('Traffic and resets', [
          ts('Bytes by service', sum('aws.privatelinkservices.bytes_processed', 'service_id')),
          ts('Resets sent by service', sum('aws.privatelinkservices.rst_packets_sent', 'service_id'), { display: 'bars', palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/VPN': {
    vars: ['vpnid'],
    monitor: 'amazon_vpn',
    note: 'Site-to-Site VPN connections: whether their tunnels are up (1 up, 0 down; a connection with one of its two tunnels up shows 0.5) and the data through them.',
    units: { TunnelDataIn: 'byte', TunnelDataOut: 'byte' },
    build: ({ sum, avg, min }) => ({
      kpis: [
        qv('Connections', avg('aws.vpn.tunnel_state', 'vpnid'), { formula: 'count_not_null(query1)' }),
        qv('Lowest tunnel state', min('aws.vpn.tunnel_state'), { agg: 'min', precision: 1 }),
        qv('Avg tunnel state', avg('aws.vpn.tunnel_state'), { precision: 2 }),
        qv('Data in', sum('aws.vpn.tunnel_data_in')),
        qv('Data out', sum('aws.vpn.tunnel_data_out')),
      ],
      groups: [
        group('Tunnel state', [
          ts('Tunnel state by connection', avg('aws.vpn.tunnel_state', 'vpnid'), { w: 8 }),
          top('Connections with tunnels down', min('aws.vpn.tunnel_state', 'vpnid'), { order: 'asc', w: 4, palette: 'warm' }),
        ]),
        group('Traffic', [
          ts('Data in and out', [sum('aws.vpn.tunnel_data_in'), sum('aws.vpn.tunnel_data_out')], {
            formulas: [
              ['query1', 'in'],
              ['0 - query2', 'out'],
            ],
            display: 'bars',
            w: 8,
          }),
          top('Busiest connections', sum('aws.vpn.tunnel_data_in', 'vpnid'), { w: 4 }),
        ]),
      ],
    }),
  },

  'AWS/Events': {
    vars: ['rulename'],
    monitor: 'amazon_event_bridge',
    note: 'EventBridge rules: how often they match, the target invocations they make and those that fail or are throttled, and the events sent to dead-letter queues.',
    build: ({ sum }) => ({
      kpis: [
        qv('Rules triggered', sum('aws.events.triggered_rules')),
        qv('Invocations', sum('aws.events.invocations')),
        qv('Failed invocations', sum('aws.events.failed_invocations'), { bad: true }),
        qv('Throttled', sum('aws.events.throttled_rules'), { bad: true }),
        qv('Sent to dead-letter queues', sum('aws.events.invocations_sent_to_dlq')),
        qv('Lost (dead-letter queue failed)', sum('aws.events.invocations_failed_to_be_sent_to_dlq'), { bad: true }),
      ],
      groups: [
        group('Rules and invocations', [
          ts('Matches by rule', sum('aws.events.triggered_rules', 'rulename'), { display: 'bars' }),
          ts('Invocations by rule', sum('aws.events.invocations', 'rulename')),
          top('Most triggered rules', sum('aws.events.triggered_rules', 'rulename')),
          ts('Failure rate by rule', [sum('aws.events.failed_invocations', 'rulename'), sum('aws.events.invocations', 'rulename')], {
            formulas: [['100 * query1 / query2', 'failed %']],
            palette: 'warm',
          }),
        ]),
        group('Failures', [
          ts('Failed invocations by rule', sum('aws.events.failed_invocations', 'rulename'), { display: 'bars', palette: 'warm' }),
          ts('Throttled rules', sum('aws.events.throttled_rules', 'rulename'), { display: 'bars', palette: 'warm' }),
        ]),
        group('Dead-letter queues', [
          ts('Dead-letter queue deliveries', [sum('aws.events.invocations_sent_to_dlq'), sum('aws.events.invocations_failed_to_be_sent_to_dlq'), sum('aws.events.dead_letter_invocations')], {
            alias: ['sent', 'failed to send', 'dead-letter invocations'],
            palette: 'warm',
          }),
          top('Rules sending most to dead-letter queues', sum('aws.events.invocations_sent_to_dlq', 'rulename'), { palette: 'warm' }),
        ]),
      ],
    }),
  },

  'AWS/Logs': {
    vars: ['loggroupname'],
    monitor: 'amazon_logs',
    note: 'Log groups: the events and bytes they ingest, what subscription filters forward and fail to deliver, and transformation, embedded metric format and anomaly detection results.',
    logs: [['CloudWatch logs', 'source:cloudwatch']],
    units: { IncomingBytes: 'byte', TransformedBytes: 'byte' },
    build: ({ sum }) => ({
      kpis: [
        qv('Events ingested', sum('aws.logs.incoming_log_events')),
        qv('Bytes ingested', sum('aws.logs.incoming_bytes')),
        qv('Events forwarded', sum('aws.logs.forwarded_log_events')),
        qv('Delivery errors', sum('aws.logs.delivery_errors'), { bad: true }),
        qv('Delivery throttling', sum('aws.logs.delivery_throttling'), { bad: true }),
        qv('Anomalies', sum('aws.logs.anomaly_count')),
      ],
      groups: [
        group('Ingestion', [
          ts('Bytes by log group', sum('aws.logs.incoming_bytes', 'loggroupname'), { display: 'bars' }),
          ts('Events by log group', sum('aws.logs.incoming_log_events', 'loggroupname')),
          top('Largest log groups', sum('aws.logs.incoming_bytes', 'loggroupname')),
          tree('Share of ingested bytes', sum('aws.logs.incoming_bytes', 'loggroupname')),
        ]),
        group('Subscriptions', [
          ts('Events forwarded by log group', sum('aws.logs.forwarded_log_events', 'loggroupname')),
          ts('Delivery errors and throttling', [sum('aws.logs.delivery_errors'), sum('aws.logs.delivery_throttling')], { alias: ['errors', 'throttling'], palette: 'warm' }),
        ]),
        group('Processing', [
          ts('Transformed events and bytes', [sum('aws.logs.transformed_log_events'), sum('aws.logs.transformed_bytes')], { alias: ['events', 'bytes'] }),
          ts('Processing errors', [sum('aws.logs.transformation_errors'), sum('aws.logs.emfparsing_errors'), sum('aws.logs.emfvalidation_errors')], {
            alias: ['transformation', 'EMF parsing', 'EMF validation'],
            palette: 'warm',
          }),
          ts('Anomalies and sensitive data findings', [sum('aws.logs.anomaly_count'), sum('aws.logs.log_events_with_findings')], { alias: ['anomalies', 'events with findings'], w: 12 }),
        ]),
      ],
    }),
  },

  'AWS/Billing': {
    vars: ['servicename', 'currency'],
    monitor: 'amazon_billing',
    note: 'Estimated charges for the month so far, per AWS service (billing alerts must be enabled on the account; CloudWatch updates them a few times a day). Cloud Cost Management reads the detailed bills.',
    units: { EstimatedCharges: 'dollar' },
    build: ({ sum, max }) => {
      const s = 'servicename:*';
      return {
        kpis: [
          qv('Month to date', sum('aws.billing.estimated_charges', null, s), { agg: 'last', span: '1w', precision: 2 }),
          qv('Services billed', max('aws.billing.estimated_charges', 'servicename', s), { formula: 'count_not_null(query1)', span: '1w' }),
          qv('Largest service', max('aws.billing.estimated_charges', null, s), { agg: 'last', span: '1w', precision: 2 }),
        ],
        groups: [
          group('By service', [
            top('Highest charges', max('aws.billing.estimated_charges', 'servicename', s), { agg: 'last', span: '1w', h: 3 }),
            tree('Share of the charges', max('aws.billing.estimated_charges', 'servicename', s), { agg: 'last', span: '1w', h: 3 }),
          ]),
          group('Trend', [
            ts('Charges by service', max('aws.billing.estimated_charges', 'servicename', s), { display: 'area', span: '1mo', w: 8, h: 3 }),
            chg('Against the week before', max('aws.billing.estimated_charges', 'servicename', s), { increaseGood: false, w: 4, h: 3 }),
          ]),
        ],
      };
    },
  },

  'AWS/AutoScaling': {
    vars: ['autoscalinggroupname'],
    monitor: 'amazon_auto_scaling',
    note: 'Auto Scaling groups: how many instances each wants, has in service and is still launching or terminating, against its minimum and maximum size. Group metrics must be enabled on each group to reach CloudWatch.',
    build: ({ sum, avg, max }) => {
      const g = 'autoscalinggroupname';
      return {
        kpis: [
          qv('Groups reporting', avg('aws.autoscaling.group_desired_capacity', g), { formula: 'count_not_null(query1)', good: true }),
          qv('Instances in service', sum('aws.autoscaling.group_in_service_instances'), { agg: 'last' }),
          qv('Desired capacity', sum('aws.autoscaling.group_desired_capacity'), { agg: 'last' }),
          qv('Pending', sum('aws.autoscaling.group_pending_instances'), { agg: 'last' }),
          qv('Terminating', sum('aws.autoscaling.group_terminating_instances'), { agg: 'last' }),
        ],
        groups: [
          group('Desired against in service', [
            ts('In service and desired', [sum('aws.autoscaling.group_in_service_instances'), sum('aws.autoscaling.group_desired_capacity')], { alias: ['in service', 'desired'] }),
            ts('Missing instances by group', [sum('aws.autoscaling.group_desired_capacity', g), sum('aws.autoscaling.group_in_service_instances', g)], {
              formulas: [['query1 - query2', 'desired − in service']],
              palette: 'warm',
            }),
            tbl('Groups', [
              ['Min', avg('aws.autoscaling.group_min_size', g), 'last'],
              ['Desired', avg('aws.autoscaling.group_desired_capacity', g), 'last'],
              ['In service', avg('aws.autoscaling.group_in_service_instances', g), 'last'],
              ['Max', avg('aws.autoscaling.group_max_size', g), 'last'],
            ]),
          ]),
          group('Scaling activity', [
            ts('Instances launching', sum('aws.autoscaling.group_pending_instances', g), { display: 'bars', w: 4 }),
            ts('Instances terminating', sum('aws.autoscaling.group_terminating_instances', g), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Instances in standby', sum('aws.autoscaling.group_standby_instances', g), { display: 'bars', w: 4 }),
            ts('Total instances by group', sum('aws.autoscaling.group_total_instances', g), { display: 'area' }),
            chg('Size against the week before', sum('aws.autoscaling.group_total_instances', g)),
          ]),
          group('Headroom', [
            top('Closest to their maximum', [max('aws.autoscaling.group_desired_capacity', g), max('aws.autoscaling.group_max_size', g)], { formula: '100 * query1 / query2', agg: 'last' }),
            tree('Instances by group', sum('aws.autoscaling.group_in_service_instances', g), { agg: 'last' }),
          ]),
        ],
      };
    },
  },

  'AWS/DocDB': {
    vars: ['dbinstanceidentifier'],
    monitor: 'amazon_documentdb',
    note: 'DocumentDB instances: CPU, open connections, read and write throughput and how long each engine has been up (a reset uptime is a restart or failover).',
    units: { CPUUtilization: 'percent', ReadThroughput: 'byte', WriteThroughput: 'byte', EngineUptime: 'second' },
    logs: [['DocumentDB logs', 'source:docdb']],
    build: ({ sum, avg, max, min }) => {
      const i = 'dbinstanceidentifier';
      return {
        kpis: [
          qv('Instances reporting', avg('aws.docdb.cpuutilization', i), { formula: 'count_not_null(query1)', good: true }),
          qv('Avg CPU', avg('aws.docdb.cpuutilization'), { precision: 1 }),
          qv('Peak CPU', max('aws.docdb.cpuutilization.maximum'), { precision: 1 }),
          qv('Connections', sum('aws.docdb.database_connections'), { agg: 'last' }),
          qv('Bytes read', sum('aws.docdb.read_throughput')),
          qv('Bytes written', sum('aws.docdb.write_throughput')),
        ],
        groups: [
          group('Instance load', [
            top('Busiest instances', avg('aws.docdb.cpuutilization', i), { w: 4, h: 4, palette: 'orange' }),
            ts('CPU by instance', avg('aws.docdb.cpuutilization', i), { w: 8 }),
            ts('Connections by instance', avg('aws.docdb.database_connections', i), { w: 8 }),
          ]),
          group('Document I/O', [
            ts('Read and write throughput', [sum('aws.docdb.read_throughput'), sum('aws.docdb.write_throughput')], { alias: ['read', 'written'], display: 'bars' }),
            top('Most written instances', sum('aws.docdb.write_throughput', i)),
          ]),
          group('Restarts', [top('Most recently restarted', min('aws.docdb.engine_uptime', i), { order: 'asc', agg: 'last' }), ts('Engine uptime by instance', min('aws.docdb.engine_uptime', i))]),
        ],
      };
    },
  },

  'AWS/MemoryDB': {
    vars: ['clustername'],
    monitor: 'amazon_memorydb',
    note: 'MemoryDB clusters: engine and host CPU, memory used against what is left, cache hits and evictions, connections, replication lag, network allowance breaches and failed authentications.',
    units: {
      CPUUtilization: 'percent',
      EngineCPUUtilization: 'percent',
      DatabaseMemoryUsagePercentage: 'percent',
      BytesUsedForMemoryDB: 'byte',
      FreeableMemory: 'byte',
      SwapUsage: 'byte',
      NetworkBytesIn: 'byte',
      NetworkBytesOut: 'byte',
      ReplicationBytes: 'byte',
      ReplicationLag: 'second',
    },
    build: ({ sum, avg, max }) => {
      const c = 'clustername';
      return {
        kpis: [
          qv('Busiest engine CPU', max('aws.memorydb.engine_cpuutilization.maximum'), { precision: 1 }),
          qv('Memory used', max('aws.memorydb.database_memory_usage_percentage'), { precision: 1, unit: '%' }),
          qv('Hit rate', [sum('aws.memorydb.keyspace_hits'), sum('aws.memorydb.keyspace_misses')], { formula: '100 * query1 / (query1 + query2)', precision: 1, unit: '%' }),
          qv('Evictions', sum('aws.memorydb.evictions'), { bad: true }),
          qv('Connections', sum('aws.memorydb.curr_connections'), { agg: 'last' }),
          qv('Auth failures', sum('aws.memorydb.authentication_failures'), { bad: true }),
        ],
        groups: [
          group('Engine and host CPU', [ts('Engine CPU by cluster', avg('aws.memorydb.engine_cpuutilization', c)), ts('Host CPU by cluster', avg('aws.memorydb.cpuutilization', c))]),
          group('Memory and keys', [
            top('Fullest clusters', max('aws.memorydb.database_memory_usage_percentage', c), { w: 4 }),
            ts('Memory used and freeable', [sum('aws.memorydb.bytes_used_for_memory_db'), sum('aws.memorydb.freeable_memory')], { alias: ['used', 'freeable'], w: 4 }),
            ts('Fragmentation ratio', avg('aws.memorydb.memory_fragmentation_ratio', c), { w: 4 }),
            ts('Hits and misses', [sum('aws.memorydb.keyspace_hits'), sum('aws.memorydb.keyspace_misses')], { alias: ['hits', 'misses'], display: 'bars' }),
            ts('Evictions and reclaimed keys', [sum('aws.memorydb.evictions'), sum('aws.memorydb.reclaimed')], { alias: ['evicted', 'expired'], palette: 'warm' }),
            ts('Items by cluster', sum('aws.memorydb.curr_items', c), { w: 12 }),
          ]),
          group('Clients and replication', [
            ts('Connections by cluster', sum('aws.memorydb.curr_connections', c), { w: 4 }),
            ts('New connections', sum('aws.memorydb.new_connections', c), { display: 'bars', w: 4 }),
            ts('Replication lag', max('aws.memorydb.replication_lag.maximum', c), { w: 4 }),
          ]),
          group('Network allowances and access', [
            ts(
              'Packets shaped for exceeding allowances',
              [
                sum('aws.memorydb.network_bandwidth_in_allowance_exceeded.sum'),
                sum('aws.memorydb.network_bandwidth_out_allowance_exceeded.sum'),
                sum('aws.memorydb.network_packets_per_second_allowance_exceeded.sum'),
                sum('aws.memorydb.network_conntrack_allowance_exceeded.sum'),
              ],
              { alias: ['bandwidth in', 'bandwidth out', 'packets/s', 'connections tracked'], palette: 'warm', display: 'bars' },
            ),
            ts(
              'Authentication and authorization failures',
              [sum('aws.memorydb.authentication_failures'), sum('aws.memorydb.command_authorization_failures'), sum('aws.memorydb.key_authorization_failures')],
              { alias: ['authentication', 'commands', 'keys'], palette: 'warm', display: 'bars' },
            ),
          ]),
        ],
      };
    },
  },

  Glue: {
    vars: ['jobname'],
    monitor: 'amazon_glue',
    note: 'Glue ETL jobs (with job metrics enabled): tasks completed and failed, records and bytes read, S3 traffic, executors against what the job needs, and driver and executor memory. Totals read each job’s ALL series, not its individual runs.',
    units: {
      'glue.driver.aggregate.bytesRead': 'byte',
      'glue.driver.aggregate.elapsedTime': 'millisecond',
      'glue.driver.aggregate.shuffleBytesWritten': 'byte',
      'glue.driver.s3.filesystem.read_bytes': 'byte',
      'glue.ALL.s3.filesystem.read_bytes': 'byte',
      'glue.driver.s3.filesystem.write_bytes': 'byte',
      'glue.ALL.s3.filesystem.write_bytes': 'byte',
      'glue.driver.streaming.batchProcessingTimeInMs': 'millisecond',
      'glue.driver.jvm.heap.usage': 'fraction',
      'glue.ALL.jvm.heap.usage': 'fraction',
      'glue.driver.jvm.heap.used': 'byte',
      'glue.ALL.jvm.heap.used': 'byte',
      'glue.driver.system.cpuSystemLoad': 'fraction',
      'glue.ALL.system.cpuSystemLoad': 'fraction',
    },
    logs: [['Glue job logs', 'source:glue']],
    build: ({ sum, avg, max }) => {
      const j = 'jobname';
      const all = 'jobrunid:ALL';
      const n = (m) => `aws.glue.glue_${m}`;
      return {
        kpis: [
          qv('Tasks completed', sum(n('driver_aggregate_num_completed_tasks'), null, all)),
          qv('Tasks failed', sum(n('driver_aggregate_num_failed_tasks'), null, all), { bad: true }),
          qv('Tasks killed', sum(n('driver_aggregate_num_killed_tasks'), null, all), { bad: true }),
          qv('Records read', sum(n('driver_aggregate_records_read'), null, all)),
          qv('Bytes read', sum(n('driver_aggregate_bytes_read'), null, all)),
        ],
        groups: [
          group('Job progress', [
            ts('Completed tasks by job', sum(n('driver_aggregate_num_completed_tasks'), j, all), { display: 'bars' }),
            ts('Failed and killed tasks', [sum(n('driver_aggregate_num_failed_tasks'), null, all), sum(n('driver_aggregate_num_killed_tasks'), null, all)], {
              alias: ['failed', 'killed'],
              palette: 'warm',
              display: 'bars',
            }),
            top('Longest running jobs', sum(n('driver_aggregate_elapsed_time'), j, all)),
            top('Most failed tasks', sum(n('driver_aggregate_num_failed_tasks'), j, all), { palette: 'warm' }),
          ]),
          group('Data movement', [
            ts('S3 read and written', [sum(n('all_s3_filesystem_read_bytes'), null, all), sum(n('all_s3_filesystem_write_bytes'), null, all)], { alias: ['read', 'written'], w: 4 }),
            ts('Records read by job', sum(n('driver_aggregate_records_read'), j, all), { w: 4 }),
            ts('Shuffle bytes written', sum(n('driver_aggregate_shuffle_bytes_written'), j, all), { w: 4 }),
          ]),
          group('Executors and memory', [
            ts(
              'Executors running and needed',
              [avg(n('driver_executor_allocation_manager_executors_number_all_executors'), null, all), max(n('driver_executor_allocation_manager_executors_number_max_needed_executors'), null, all)],
              {
                alias: ['running', 'needed'],
              },
            ),
            ts('Heap usage, driver and executors', [max(n('driver_jvm_heap_usage'), null, all), max(n('all_jvm_heap_usage'), null, all)], { alias: ['driver', 'all executors'] }),
            ts('CPU load by job', avg(n('all_system_cpu_system_load'), j, all), { w: 8 }),
            ts('Streaming batch time', avg(n('driver_streaming_batch_processing_time_in_ms'), j, all), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'AWS/ElasticMapReduce': {
    vars: ['jobflowid'],
    monitor: 'amazon_emr',
    note: 'EMR clusters: YARN applications submitted, running and failed, containers and memory against what is available, node counts, HDFS health and use, and S3 traffic. An idle cluster (IsIdle) is often one to shut down.',
    units: {
      HDFSBytesRead: 'byte',
      HDFSBytesWritten: 'byte',
      S3BytesRead: 'byte',
      S3BytesWritten: 'byte',
      HDFSUtilization: 'percent',
      YARNMemoryAvailablePercentage: 'percent',
      MemoryAllocatedMB: 'mebibyte',
      MemoryReservedMB: 'mebibyte',
      MemoryTotalMB: 'mebibyte',
      CapacityRemainingGB: 'gibibyte',
    },
    build: ({ sum, avg, min, max }) => {
      const c = 'jobflowid';
      const e = (m) => `aws.elasticmapreduce.${m}`;
      return {
        kpis: [
          qv('Clusters reporting', avg(e('is_idle'), c), { formula: 'count_not_null(query1)', good: true }),
          qv('Idle clusters', sum(e('is_idle')), { agg: 'last' }),
          qv('Apps running', sum(e('apps_running')), { agg: 'last' }),
          qv('Apps failed', sum(e('apps_failed')), { bad: true }),
          qv('Lowest YARN memory free', min(e('yarnmemory_available_percentage')), { precision: 1 }),
          qv('Missing HDFS blocks', max(e('missing_blocks')), { bad: true }),
        ],
        groups: [
          group('YARN applications', [
            ts('Applications by state', [sum(e('apps_running')), sum(e('apps_pending')), sum(e('apps_completed')), sum(e('apps_failed')), sum(e('apps_killed'))], {
              alias: ['running', 'pending', 'completed', 'failed', 'killed'],
              display: 'bars',
            }),
            top('Most failed applications', sum(e('apps_failed'), c), { palette: 'warm' }),
            ts('Containers allocated and pending', [sum(e('container_allocated')), sum(e('container_pending'))], { alias: ['allocated', 'pending'] }),
            ts('YARN memory free by cluster', min(e('yarnmemory_available_percentage'), c)),
          ]),
          group('Cluster nodes', [
            ts('Core and task nodes running', [sum(e('core_nodes_running')), sum(e('task_nodes_running'))], { alias: ['core', 'task'], w: 4 }),
            ts('Nodes waiting to start', [sum(e('core_nodes_pending')), sum(e('task_nodes_pending'))], { alias: ['core', 'task'], w: 4 }),
            ts('Unhealthy and lost nodes', [sum(e('mrunhealthy_nodes')), sum(e('mrlost_nodes'))], { alias: ['unhealthy', 'lost'], palette: 'warm', w: 4 }),
            tbl('Clusters', [
              ['Idle', avg(e('is_idle'), c), 'last'],
              ['Apps running', sum(e('apps_running'), c), 'last'],
              ['YARN memory free %', min(e('yarnmemory_available_percentage'), c)],
              ['HDFS used %', max(e('hdfsutilization'), c)],
            ]),
          ]),
          group('HDFS and S3', [
            ts('HDFS used by cluster', max(e('hdfsutilization'), c), { w: 4 }),
            ts('HDFS block problems', [max(e('missing_blocks')), sum(e('corrupt_blocks')), sum(e('under_replicated_blocks'))], {
              alias: ['missing', 'corrupt', 'under-replicated'],
              palette: 'warm',
              w: 4,
            }),
            ts('Live data nodes', sum(e('live_data_nodes'), c), { w: 4 }),
            ts('S3 traffic', [sum(e('s3bytes_read')), sum(e('s3bytes_written'))], { alias: ['read', 'written'], display: 'bars' }),
            ts('HDFS traffic', [sum(e('hdfsbytes_read')), sum(e('hdfsbytes_written'))], { alias: ['read', 'written'], display: 'bars' }),
          ]),
        ],
      };
    },
  },

  '/aws/sagemaker/Endpoints': {
    vars: ['endpointname', 'variantname'],
    monitor: 'amazon_sagemaker',
    note: 'SageMaker inference endpoints, per production variant: CPU, GPU, GPU memory, memory and disk of the instances serving them (CPU and GPU sum over cores: 100 per core), and what each variant reserves.',
    units: {
      CPUUtilization: 'percent',
      CPUUtilizationNormalized: 'percent',
      CPUReservation: 'percent',
      MemoryUtilization: 'percent',
      MemoryReservation: 'percent',
      DiskUtilization: 'percent',
      GPUUtilization: 'percent',
      GPUUtilizationNormalized: 'percent',
      GPUMemoryUtilization: 'percent',
      GPUMemoryUtilizationNormalized: 'percent',
      GPUReservation: 'percent',
    },
    build: ({ avg, max }) => {
      const s = (m) => `aws.sagemaker_endpoints.${m}`;
      const v = 'endpointname,variantname';
      return {
        kpis: [
          qv('Variants reporting', avg(s('cpuutilization'), v), { formula: 'count_not_null(query1)', good: true }),
          qv('Busiest CPU (normalized)', max(s('cpuutilization_normalized')), { precision: 1 }),
          qv('Busiest GPU (normalized)', max(s('gpuutilization_normalized')), { precision: 1 }),
          qv('Highest memory', max(s('memory_utilization')), { precision: 1 }),
          qv('Fullest disk', max(s('disk_utilization')), { precision: 1 }),
        ],
        groups: [
          group('Variant CPU and memory', [
            top('Busiest variants', avg(s('cpuutilization_normalized'), v), { w: 4, h: 4, palette: 'orange' }),
            ts('CPU by variant (normalized)', avg(s('cpuutilization_normalized'), v), { w: 8 }),
            ts('Memory by variant', avg(s('memory_utilization'), v), { w: 8 }),
          ]),
          group('Accelerators', [ts('GPU by variant (normalized)', avg(s('gpuutilization_normalized'), v)), ts('GPU memory by variant (normalized)', avg(s('gpumemory_utilization_normalized'), v))]),
          group('Reservations and disk', [
            tbl('Variants', [
              ['CPU reserved %', avg(s('cpureservation'), v)],
              ['Memory reserved %', avg(s('memory_reservation'), v)],
              ['GPU reserved %', avg(s('gpureservation'), v)],
              ['Disk used %', max(s('disk_utilization'), v)],
            ]),
          ]),
        ],
      };
    },
  },

  '/aws/sagemaker/TrainingJobs': {
    vars: ['host'],
    monitor: 'amazon_sagemaker',
    note: 'SageMaker training jobs, per training host (job name and instance, e.g. algo-1): CPU, GPU, GPU memory, memory and disk, to see whether a job uses the instances it pays for.',
    units: { CPUUtilization: 'percent', MemoryUtilization: 'percent', DiskUtilization: 'percent', GPUUtilization: 'percent', GPUMemoryUtilization: 'percent' },
    build: ({ avg, max }) => {
      const s = (m) => `aws.sagemaker_trainingjobs.${m}`;
      return {
        kpis: [
          qv('Hosts training', avg(s('cpuutilization'), 'host'), { formula: 'count_not_null(query1)', good: true }),
          qv('Avg GPU', avg(s('gpuutilization')), { precision: 1 }),
          qv('Avg CPU', avg(s('cpuutilization')), { precision: 1 }),
          qv('Highest memory', max(s('memory_utilization')), { precision: 1 }),
        ],
        groups: [
          group('Training hosts', [
            ts('GPU by host', avg(s('gpuutilization'), 'host')),
            ts('GPU memory by host', avg(s('gpumemory_utilization'), 'host')),
            heat('CPU spread across hosts', avg(s('cpuutilization'), 'host')),
            top('Least busy GPUs', avg(s('gpuutilization'), 'host'), { order: 'asc' }),
          ]),
          group('Host memory and scratch disk', [ts('Memory by host', max(s('memory_utilization'), 'host')), top('Fullest disks', max(s('disk_utilization'), 'host'))]),
        ],
      };
    },
  },

  '/aws/sagemaker/ProcessingJobs': {
    vars: ['host'],
    monitor: 'amazon_sagemaker',
    note: 'SageMaker processing jobs (data preparation, evaluation), per processing host: CPU, memory, disk and, on accelerated instances, GPU and GPU memory.',
    units: { CPUUtilization: 'percent', MemoryUtilization: 'percent', DiskUtilization: 'percent', GPUUtilization: 'percent', GPUMemoryUtilization: 'percent' },
    build: ({ avg, max }) => {
      const s = (m) => `aws.sagemaker_processingjobs.${m}`;
      return {
        kpis: [
          qv('Hosts processing', avg(s('cpuutilization'), 'host'), { formula: 'count_not_null(query1)', good: true }),
          qv('Avg CPU', avg(s('cpuutilization')), { precision: 1 }),
          qv('Highest memory', max(s('memory_utilization')), { precision: 1 }),
          qv('Fullest disk', max(s('disk_utilization')), { precision: 1 }),
        ],
        groups: [
          group('Processing hosts', [
            top('Busiest hosts', avg(s('cpuutilization'), 'host'), { w: 4, palette: 'orange' }),
            ts('CPU by processing host', avg(s('cpuutilization'), 'host'), { w: 8 }),
            ts('Memory by processing host', max(s('memory_utilization'), 'host')),
            ts('Disk by processing host', max(s('disk_utilization'), 'host')),
          ]),
          group('GPU processing', [ts('GPU by processing host', avg(s('gpuutilization'), 'host')), ts('GPU memory by processing host', avg(s('gpumemory_utilization'), 'host'))]),
        ],
      };
    },
  },

  'AWS/TransitGateway': {
    vars: ['transitgateway'],
    monitor: 'amazon_transit_gateway',
    note: 'Transit gateways: bytes and packets between attached VPCs, VPNs and Direct Connect gateways, and the packets dropped for a blackhole route or no route at all (usually a missing route table entry).',
    units: { BytesIn: 'byte', BytesOut: 'byte' },
    build: ({ sum }) => {
      const t = 'transitgateway';
      const tg = (m) => `aws.transitgateway.${m}`;
      return {
        kpis: [
          qv('Bytes in', sum(tg('bytes_in'))),
          qv('Bytes out', sum(tg('bytes_out'))),
          qv('Dropped: blackhole', sum(tg('packet_drop_count_blackhole')), { bad: true }),
          qv('Dropped: no route', sum(tg('packet_drop_count_no_route')), { bad: true }),
        ],
        groups: [
          group('Traffic through the gateway', [
            ts('Bytes in and out', [sum(tg('bytes_in')), sum(tg('bytes_out'))], {
              formulas: [
                ['query1', 'in'],
                ['0 - query2', 'out'],
              ],
              display: 'bars',
            }),
            ts('Packets in and out', [sum(tg('packets_in')), sum(tg('packets_out'))], { alias: ['in', 'out'] }),
            top('Busiest gateways', sum(tg('bytes_in'), t)),
            chg('Traffic against the week before', sum(tg('bytes_in'), t)),
          ]),
          group('Dropped packets', [
            ts('Drops by gateway', [sum(tg('packet_drop_count_blackhole'), t), sum(tg('packet_drop_count_no_route'), t)], { alias: ['blackhole', 'no route'], display: 'bars', palette: 'warm', w: 8 }),
            txt('A blackhole drop matches a route whose attachment is gone; a no-route drop matches no route at all. Both point to the route tables of the gateway.', { w: 4 }),
          ]),
        ],
      };
    },
  },

  'AWS/DDoSProtection': {
    vars: ['resourcearn'],
    monitor: 'amazon_shield',
    note: 'Shield Advanced: whether a protected resource is under a detected DDoS event, the attack’s size in bits, packets and requests per second by vector, and the traffic Shield saw and mitigated.',
    build: ({ sum, max }) => {
      const r = 'resourcearn';
      const d = (m) => `aws.ddosprotection.${m}`;
      return {
        kpis: [
          qv('Resources under attack', max(d('ddo_sdetected'), r), { formula: 'count_nonzero(query1)', agg: 'last', bad: true }),
          qv('Peak attack bits/s', max(d('ddo_sattack_bits_per_second'))),
          qv('Peak attack packets/s', max(d('ddo_sattack_packets_per_second'))),
          qv('Attack requests', sum(d('ddo_sattack_requests')), { bad: true }),
        ],
        groups: [
          group('Detected events', [
            ts('DDoS detected by resource', max(d('ddo_sdetected'), r), { display: 'bars', palette: 'warm', w: 8 }),
            top('Resources attacked most', max(d('ddo_sdetected'), r), { palette: 'warm', w: 4 }),
          ]),
          group('Attack size by vector', [
            ts('Bits per second', max(d('ddo_sattack_bits_per_second'), 'attackvector'), { w: 4 }),
            ts('Packets per second', max(d('ddo_sattack_packets_per_second'), 'attackvector'), { w: 4 }),
            ts('Requests per second (layer 7)', max(d('ddo_sattack_requests_per_second'), 'attackvector'), { w: 4 }),
          ]),
          group('Traffic seen and mitigated', [
            ts('Packets per second by mitigation', max(d('volume_packets_per_second'), 'mitigationaction')),
            ts('Bits per second by protocol', max(d('volume_bits_per_second'), 'protocol')),
          ]),
        ],
      };
    },
  },

  'AWS/RDS/Proxy': {
    vars: ['proxyname'],
    monitor: 'amazon_rds_proxy',
    note: 'RDS Proxy: client connections against the database connections it pools, how many are borrowed, pinned to a session or in a transaction, query volume and latency, failed connection setups and the availability of its targets.',
    build: ({ sum, avg, max, min }) => {
      const p = 'proxyname';
      const x = (m) => `aws.rds.proxy.${m}`;
      return {
        kpis: [
          qv('Client connections', sum(x('client_connections')), { agg: 'last' }),
          qv('Database connections', sum(x('database_connections')), { agg: 'last' }),
          qv('Queries', sum(x('query_requests'))),
          qv('Avg query latency', avg(x('query_response_latency')), { precision: 0 }),
          qv('Lowest target availability', min(x('availability_percentage')), { precision: 1 }),
          qv('Failed auth', sum(x('client_connections_setup_failed_auth')), { bad: true }),
        ],
        groups: [
          group('Client side', [
            ts('Client connections by proxy', sum(x('client_connections'), p), { w: 4 }),
            ts('Connections received and over TLS', [sum(x('client_connections_received')), sum(x('client_connections_tls'))], { alias: ['received', 'TLS'], w: 4 }),
            ts('Failed authentications', sum(x('client_connections_setup_failed_auth'), p), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
          group('Connection pool', [
            ts('Database connections and the limit', [sum(x('database_connections')), sum(x('max_database_connections_allowed'))], { alias: ['open', 'allowed'] }),
            ts(
              'Borrowed, pinned and in a transaction',
              [sum(x('database_connections_currently_borrowed')), sum(x('database_connections_currently_session_pinned')), sum(x('database_connections_currently_in_transaction'))],
              { alias: ['borrowed', 'pinned', 'in transaction'] },
            ),
            top('Fullest pools', [max(x('database_connections'), p), max(x('max_database_connections_allowed'), p)], { formula: '100 * query1 / query2' }),
            ts('Borrow latency by proxy', avg(x('database_connections_borrow_latency'), p)),
          ]),
          group('Queries through the proxy', [
            ts('Queries by proxy', sum(x('query_requests'), p), { display: 'bars', w: 4 }),
            ts('Response latency: proxy and database', [avg(x('query_response_latency')), avg(x('query_database_response_latency'))], { alias: ['through the proxy', 'database'], w: 4 }),
            ts('Failed database connection setups', sum(x('database_connections_setup_failed'), p), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Target availability', min(x('availability_percentage'), 'targetgroup'), { w: 12 }),
          ]),
        ],
      };
    },
  },

  WAF: {
    vars: ['webacl'],
    monitor: 'amazon_waf',
    note: 'WAF Classic web ACLs: requests allowed, blocked, counted and passed (totals are each ACL’s ALL rule), and the rules that block most. WAF Classic is retired in favour of AWS WAF (its own dashboard).',
    logs: [['WAF logs', 'source:waf']],
    build: ({ sum }) => {
      const all = 'rule:ALL';
      const w = (m) => `aws.waf.${m}`;
      return {
        kpis: [
          qv('Allowed', sum(w('allowed_requests'), null, all), { good: true }),
          qv('Blocked', sum(w('blocked_requests'), null, all), { bad: true }),
          qv('Counted', sum(w('counted_requests'), null, all)),
          qv('Block rate', [sum(w('blocked_requests'), null, all), sum(w('allowed_requests'), null, all)], { formula: '100 * query1 / (query1 + query2)', precision: 2, unit: '%' }),
        ],
        groups: [
          group('Web ACL verdicts', [
            ts('Allowed and blocked', [sum(w('allowed_requests'), null, all), sum(w('blocked_requests'), null, all)], { alias: ['allowed', 'blocked'], display: 'bars' }),
            ts('Counted and passed', [sum(w('counted_requests'), null, all), sum(w('passed_requests'), null, all)], { alias: ['counted', 'passed'] }),
            top('Most blocked web ACLs', sum(w('blocked_requests'), 'webacl', all), { palette: 'warm' }),
            tree('Allowed requests by web ACL', sum(w('allowed_requests'), 'webacl', all)),
          ]),
          group('Classic rules', [
            top('Rules blocking most', sum(w('blocked_requests'), 'rule', '!rule:ALL'), { palette: 'warm', w: 4 }),
            ts('Blocked requests by rule', sum(w('blocked_requests'), 'rule', '!rule:ALL'), { display: 'bars', w: 8 }),
          ]),
        ],
      };
    },
  },

  'AWS/ElasticBeanstalk': {
    vars: ['environmentname'],
    monitor: 'amazon_elasticbeanstalk',
    note: 'Elastic Beanstalk environments (with enhanced health reporting): their health status (0 ok … 25 severe) and the requests their instances answer by status class.',
    build: ({ sum, max }) => {
      const e = 'environmentname';
      const b = (m) => `aws.elasticbeanstalk.${m}`;
      return {
        kpis: [
          qv('Environments reporting', max(b('environment_health'), e), { formula: 'count_not_null(query1)', good: true }),
          qv('Worst health', max(b('environment_health')), { agg: 'last', bad: true }),
          qv('2xx responses', sum(b('application_requests2xx'))),
          qv('5xx responses', sum(b('application_requests5xx')), { bad: true }),
          qv('Error rate', [sum(b('application_requests5xx')), sum(b('application_requests2xx'))], { formula: '100 * query1 / (query1 + query2)', precision: 2, unit: '%' }),
        ],
        groups: [
          group('Environment health', [
            ts('Health by environment', max(b('environment_health'), e), { w: 8 }),
            top('Least healthy', max(b('environment_health'), e), { agg: 'last', palette: 'warm', w: 4 }),
            txt('Enhanced health: 0 ok, 1 info, 5 unknown, 10 no data, 15 warning, 20 degraded, 25 severe.'),
          ]),
          group('Application responses', [
            ts('Responses by class', [sum(b('application_requests2xx')), sum(b('application_requests3xx')), sum(b('application_requests4xx')), sum(b('application_requests5xx'))], {
              alias: ['2xx', '3xx', '4xx', '5xx'],
              display: 'bars',
            }),
            top('Most server errors', sum(b('application_requests5xx'), e), { palette: 'warm' }),
          ]),
        ],
      };
    },
  },

  'AWS/Athena': {
    vars: ['workgroup'],
    monitor: 'amazon_athena',
    note: 'Athena workgroups: the data each workgroup scans (what queries are billed on), where a query’s time goes (queueing, planning, the engine, writing results) and, for capacity reservations, DPUs allocated against consumed.',
    units: {
      ProcessedBytes: 'byte',
      TotalExecutionTime: 'millisecond',
      EngineExecutionTime: 'millisecond',
      QueryPlanningTime: 'millisecond',
      QueryQueueTime: 'millisecond',
      ServicePreProcessingTime: 'millisecond',
      ServiceProcessingTime: 'millisecond',
    },
    build: ({ sum, avg, max }) => {
      const w = 'workgroup';
      const a = (m) => `aws.athena.${m}`;
      return {
        kpis: [
          qv('Data scanned', sum(a('processed_bytes'))),
          qv('Avg query time', avg(a('total_execution_time'))),
          qv('Slowest query', max(a('total_execution_time.maximum'))),
          qv('Avg queue time', avg(a('query_queue_time'))),
        ],
        groups: [
          group('Data scanned', [ts('Bytes scanned by workgroup', sum(a('processed_bytes'), w), { display: 'bars' }), tree('Share of bytes scanned', sum(a('processed_bytes'), w))]),
          group('Where query time goes', [
            ts('Query phases', [avg(a('query_queue_time')), avg(a('query_planning_time')), avg(a('engine_execution_time')), avg(a('service_processing_time'))], {
              alias: ['queued', 'planning', 'engine', 'writing results'],
              display: 'area',
            }),
            top('Slowest workgroups', avg(a('total_execution_time'), w)),
          ]),
          group('Capacity reservations', [
            ts('DPUs allocated and consumed', [sum(a('dpuallocated')), sum(a('dpuconsumed'))], { alias: ['allocated', 'consumed'], w: 8 }),
            top('DPUs per query', max(a('dpucount'), w), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'AWS/AmazonMQ': {
    vars: ['broker'],
    monitor: 'amazon_mq',
    note: 'Amazon MQ brokers (RabbitMQ engine metrics; ActiveMQ brokers publish per-queue metrics too): messages ready and unacknowledged, publish and acknowledge rates, consumers and producers, connections and CPU.',
    units: { CpuUtilization: 'percent', NetworkIn: 'byte', NetworkOut: 'byte' },
    build: ({ sum, avg, max }) => {
      const b = 'broker';
      const q = (m) => `aws.amazonmq.${m}`;
      return {
        kpis: [
          qv('Messages ready', sum(q('message_ready_count')), { agg: 'last' }),
          qv('Unacknowledged', sum(q('message_unacknowledged_count')), { agg: 'last' }),
          qv('Publish rate', sum(q('publish_rate')), { precision: 1 }),
          qv('Consumers', sum(q('consumer_count')), { agg: 'last' }),
          qv('Busiest broker CPU', max(q('cpu_utilization')), { precision: 1 }),
        ],
        groups: [
          group('Message backlog', [
            ts('Ready and unacknowledged', [sum(q('message_ready_count')), sum(q('message_unacknowledged_count'))], { alias: ['ready', 'unacknowledged'], display: 'area' }),
            top('Largest backlogs', sum(q('message_count'), b), { agg: 'last' }),
            ts('Publish, confirm and ack rates', [avg(q('publish_rate')), avg(q('confirm_rate')), avg(q('ack_rate'))], { alias: ['published', 'confirmed', 'acknowledged'], w: 12 }),
          ]),
          group('Clients', [
            ts('Connections and channels', [sum(q('connection_count')), sum(q('channel_count'))], { alias: ['connections', 'channels'], w: 4 }),
            ts('Consumers by broker', sum(q('consumer_count'), b), { w: 4 }),
            tbl(
              'Brokers',
              [
                ['Queues', sum(q('queue_count'), b), 'last'],
                ['Exchanges', sum(q('exchange_count'), b), 'last'],
                ['Consumers', sum(q('consumer_count'), b), 'last'],
              ],
              { w: 4, h: 2 },
            ),
          ]),
          group('Broker hosts', [
            ts('CPU by broker', avg(q('cpu_utilization'), b)),
            ts('Network in and out', [sum(q('network_in')), sum(q('network_out'))], {
              formulas: [
                ['query1', 'in'],
                ['0 - query2', 'out'],
              ],
              display: 'bars',
            }),
          ]),
        ],
      };
    },
  },

  'AWS/Neptune': {
    vars: ['dbclusteridentifier'],
    monitor: 'amazon_neptune',
    note: 'Neptune graph clusters: Gremlin and SPARQL requests and errors, bulk loader activity, CPU and freeable memory, local storage left and the cluster volume’s size and I/O.',
    units: { CPUUtilization: 'percent', FreeableMemory: 'byte', FreeLocalStorage: 'byte', VolumeBytesUsed: 'byte', NetworkReceiveThroughput: 'byte' },
    build: ({ sum, avg, min, max }) => {
      const c = 'dbclusteridentifier';
      const n = (m) => `aws.neptune.${m}`;
      return {
        kpis: [
          qv('Gremlin requests', sum(n('gremlin_requests'))),
          qv('Gremlin errors', sum(n('gremlin_errors')), { bad: true }),
          qv('SPARQL errors', sum(n('sparql_errors')), { bad: true }),
          qv('Peak CPU', max(n('cpuutilization.maximum')), { precision: 1 }),
          qv('Least memory free', min(n('freeable_memory'))),
          qv('Volume used', sum(n('volume_bytes_used')), { agg: 'last' }),
        ],
        groups: [
          group('Graph queries', [
            ts('Gremlin and SPARQL requests per second', [sum(n('gremlin_requests_per_sec')), sum(n('sparql_requests_per_sec'))], { alias: ['Gremlin', 'SPARQL'] }),
            ts('Query errors', [sum(n('gremlin_errors')), sum(n('sparql_errors'))], { alias: ['Gremlin', 'SPARQL'], palette: 'warm', display: 'bars' }),
            top('Busiest clusters', sum(n('gremlin_requests'), c)),
            ts('Bulk loader requests', sum(n('loader_requests'), c), { display: 'bars' }),
          ]),
          group('Cluster resources', [
            ts('CPU by cluster', avg(n('cpuutilization'), c), { w: 4 }),
            ts('Freeable memory', min(n('freeable_memory'), c), { w: 4 }),
            top('Least local storage', min(n('free_local_storage'), c), { order: 'asc', w: 4 }),
          ]),
          group('Cluster volume', [
            ts('Volume IOPS', [sum(n('volume_read_iops')), sum(n('volume_write_iops'))], { alias: ['reads', 'writes'], w: 4 }),
            ts('Volume size by cluster', max(n('volume_bytes_used'), c), { w: 4 }),
            chg('Volume growth over the week', max(n('volume_bytes_used'), c), { increaseGood: false, w: 4 }),
          ]),
        ],
      };
    },
  },

  'AWS/Cassandra': {
    vars: ['keyspace', 'tablename'],
    monitor: 'amazon_keyspaces',
    note: 'Keyspaces (for Apache Cassandra) tables: capacity consumed against provisioned, throttled reads and writes, successful request latency, user and system errors, table size and TTL deletes, and the account’s use of its capacity quotas.',
    units: {
      SuccessfulRequestLatency: 'millisecond',
      ReplicationLatency: 'millisecond',
      BillableTableSizeInBytes: 'byte',
      AccountProvisionedReadCapacityUtilization: 'percent',
      AccountProvisionedWriteCapacityUtilization: 'percent',
    },
    build: ({ sum, avg, max }) => {
      const t = 'tablename';
      const k = (m) => `aws.cassandra.${m}`;
      return {
        kpis: [
          qv('Successful requests', sum(k('successful_request_count'))),
          qv('Avg latency', avg(k('successful_request_latency')), { precision: 1 }),
          qv('Throttled', [sum(k('read_throttle_events')), sum(k('write_throttle_events'))], { formula: 'query1 + query2', bad: true }),
          qv('System errors', sum(k('system_errors')), { bad: true }),
          qv('User errors', sum(k('user_errors'))),
          qv('Table size', sum(k('billable_table_size_in_bytes')), { agg: 'last' }),
        ],
        groups: [
          group('Read and write capacity', [
            ts('Read units: consumed and provisioned', [sum(k('consumed_read_capacity_units')), sum(k('provisioned_read_capacity_units'))], { alias: ['consumed', 'provisioned'] }),
            ts('Write units: consumed and provisioned', [sum(k('consumed_write_capacity_units')), sum(k('provisioned_write_capacity_units'))], { alias: ['consumed', 'provisioned'] }),
            top('Most read tables', sum(k('consumed_read_capacity_units'), t)),
            top('Most written tables', sum(k('consumed_write_capacity_units'), t)),
          ]),
          group('Throttling and errors', [
            ts('Throttled reads and writes', [sum(k('read_throttle_events')), sum(k('write_throttle_events'))], { alias: ['reads', 'writes'], palette: 'warm', display: 'bars', w: 4 }),
            ts('Errors', [sum(k('user_errors')), sum(k('system_errors')), sum(k('conditional_check_failed_requests'))], {
              alias: ['user', 'system', 'conditional check failed'],
              palette: 'warm',
              w: 4,
            }),
            ts('Partition and connection limits hit', [sum(k('storage_partition_throughput_capacity_exceeded')), sum(k('per_connection_request_rate_exceeded'))], {
              alias: ['partition throughput', 'per connection rate'],
              palette: 'warm',
              w: 4,
            }),
          ]),
          group('Latency and storage', [
            ts('Latency by operation', avg(k('successful_request_latency'), 'operation'), { w: 4 }),
            ts('Table size', max(k('billable_table_size_in_bytes'), t), { w: 4 }),
            ts('TTL deletes', sum(k('ttldeletes'), t), { display: 'bars', w: 4 }),
          ]),
          group('Account quotas', [
            ts('Provisioned capacity used of the account quota', [max(k('account_provisioned_read_capacity_utilization')), max(k('account_provisioned_write_capacity_utilization'))], {
              alias: ['read', 'write'],
            }),
            ts('Largest table’s share of the table quota', [max(k('max_provisioned_table_read_capacity_utilization')), max(k('max_provisioned_table_write_capacity_utilization'))], {
              alias: ['read', 'write'],
            }),
          ]),
        ],
      };
    },
  },

  'AWS/AOSS': {
    vars: ['collectionname'],
    monitor: 'amazon_opensearch_serverless',
    note: 'OpenSearch Serverless collections: search and ingestion requests, their errors and latency, documents indexed and searchable, hot and S3 storage, and the OpenSearch Compute Units the account is billed for.',
    units: { IngestionDataRate: 'gibibyte', StorageUsedInS3: 'byte', StorageUsedInHot: 'byte', IngestionRequestLatency: 'second', SearchRequestLatency: 'millisecond' },
    build: ({ sum, avg, max }) => {
      const c = 'collectionname';
      const o = (m) => `aws.aoss.${m}`;
      return {
        kpis: [
          qv('Search requests', sum(o('search_request_rate'))),
          qv('Search errors', sum(o('search_request_errors')), { bad: true }),
          qv('Ingestion requests', sum(o('ingestion_request_rate'))),
          qv('Ingestion errors', sum(o('ingestion_request_errors')), { bad: true }),
          qv('Searchable documents', sum(o('searchable_documents')), { agg: 'last' }),
          qv('OCUs (indexing + search)', [max(o('indexing_ocu')), max(o('search_ocu'))], { formula: 'query1 + query2', agg: 'last' }),
        ],
        groups: [
          group('Search', [
            ts('Search requests by collection', sum(o('search_request_rate'), c), { display: 'bars', w: 4 }),
            ts('Search latency by collection', avg(o('search_request_latency'), c), { w: 4 }),
            ts('Responses by class', [sum(o('2xx')), sum(o('4xx')), sum(o('5xx'))], { alias: ['2xx', '4xx', '5xx'], w: 4 }),
          ]),
          group('Ingestion', [
            ts('Ingestion requests and errors', [sum(o('ingestion_request_success')), sum(o('ingestion_request_errors'))], { alias: ['succeeded', 'failed'], display: 'bars', w: 4 }),
            ts('Documents ingested and failed', [sum(o('ingestion_document_rate')), sum(o('ingestion_document_errors'))], { alias: ['ingested', 'failed'], w: 4 }),
            ts('Ingestion latency', avg(o('ingestion_request_latency'), c), { w: 4 }),
          ]),
          group('Collection storage', [
            ts('Hot and S3 storage', [sum(o('storage_used_in_hot')), sum(o('storage_used_in_s3'))], { alias: ['hot', 'S3'] }),
            top('Largest collections', max(o('storage_used_in_s3'), c), { agg: 'last' }),
            ts('Searchable and deleted documents', [sum(o('searchable_documents')), sum(o('deleted_documents'))], { alias: ['searchable', 'deleted'] }),
            ts('Compute units', [max(o('indexing_ocu')), max(o('search_ocu'))], { alias: ['indexing', 'search'] }),
          ]),
        ],
      };
    },
  },

  'AWS/Route53Resolver': {
    vars: ['endpointid'],
    monitor: 'amazon_route53',
    note: 'Route 53 Resolver endpoints: DNS queries forwarded into your VPCs through inbound endpoints and out to your own resolvers through outbound ones.',
    logs: [['Resolver query logs', 'source:route53']],
    build: ({ sum }) => {
      const e = 'endpointid';
      const r = (m) => `aws.route53resolver.${m}`;
      return {
        kpis: [
          qv('Inbound queries', sum(r('inbound_query_volume'))),
          qv('Outbound queries', sum(r('outbound_query_volume'))),
          qv('Outbound queries (all endpoints)', sum(r('outbound_query_aggregate_volume'))),
        ],
        groups: [
          group('Inbound endpoints', [
            ts('Inbound queries by endpoint', sum(r('inbound_query_volume'), e), { display: 'bars', w: 8 }),
            top('Busiest inbound endpoints', sum(r('inbound_query_volume'), e), { w: 4 }),
          ]),
          group('Outbound endpoints', [
            ts('Outbound queries by endpoint', sum(r('outbound_query_volume'), e), { display: 'bars', w: 8 }),
            chg('Outbound queries against the week before', sum(r('outbound_query_volume'), e), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'AWS/GlobalAccelerator': {
    vars: ['accelerator'],
    monitor: 'amazon_globalaccelerator',
    note: 'Global Accelerator: flows and bytes through each accelerator, its healthy and unhealthy endpoints, flows dropped for lack of an endpoint and the TCP resets sent by the accelerator, clients and endpoints.',
    units: { ProcessedBytesIn: 'byte', ProcessedBytesOut: 'byte' },
    build: ({ sum, max }) => {
      const a = 'accelerator';
      const x = (m) => `aws.globalaccelerator.${m}`;
      return {
        kpis: [
          qv('New flows', sum(x('new_flow_count'))),
          qv('Bytes in', sum(x('processed_bytes_in'))),
          qv('Bytes out', sum(x('processed_bytes_out'))),
          qv('Healthy endpoints', max(x('healthy_endpoint_count')), { agg: 'last', good: true }),
          qv('Unhealthy endpoints', max(x('unhealthy_endpoint_count')), { agg: 'last', bad: true }),
          qv('Flows without an endpoint', sum(x('flows_dropped_no_endpoint_found')), { bad: true }),
        ],
        groups: [
          group('Accelerator flows', [
            ts('New and active flows', [sum(x('new_flow_count')), sum(x('active_flow_count'))], { alias: ['new', 'active'] }),
            ts('Bytes in and out', [sum(x('processed_bytes_in')), sum(x('processed_bytes_out'))], {
              formulas: [
                ['query1', 'in'],
                ['0 - query2', 'out'],
              ],
              display: 'bars',
            }),
            top('Busiest accelerators', sum(x('processed_bytes_out'), a)),
            ts('Packets processed', sum(x('packets_processed'), a)),
          ]),
          group('Endpoint health', [
            ts('Healthy and unhealthy endpoints', [max(x('healthy_endpoint_count')), max(x('unhealthy_endpoint_count'))], { alias: ['healthy', 'unhealthy'], w: 8 }),
            top('Accelerators with unhealthy endpoints', max(x('unhealthy_endpoint_count'), a), { palette: 'warm', w: 4 }),
          ]),
          group('Resets and drops', [
            ts('TCP resets by origin', [sum(x('tcp_aga_reset_count')), sum(x('tcp_client_reset_count')), sum(x('tcp_endpoint_reset_count'))], {
              alias: ['accelerator', 'client', 'endpoint'],
              palette: 'warm',
            }),
            ts('Flows dropped: no endpoint', sum(x('flows_dropped_no_endpoint_found'), a), { display: 'bars', palette: 'warm' }),
          ]),
        ],
      };
    },
  },

  'AWS/DX': {
    vars: ['connectionid', 'virtualinterfaceid'],
    monitor: 'amazon_directconnect',
    note: 'Direct Connect: whether each connection is up (1) or down, its bit and packet rates both ways, CRC and other errors, optical light levels, MACsec encryption, and each virtual interface’s traffic and BGP sessions.',
    units: { ConnectionBpsEgress: 'bit', ConnectionBpsIngress: 'bit', VirtualInterfaceBpsEgress: 'bit', VirtualInterfaceBpsIngress: 'bit' },
    build: ({ sum, avg, min, max }) => {
      const c = 'connectionid';
      const v = 'virtualinterfaceid';
      const d = (m) => `aws.dx.${m}`;
      return {
        kpis: [
          qv('Connections up', sum(d('connection_state')), { agg: 'last', good: true }),
          qv('Connections down', min(d('connection_state'), c), { formula: 'count_not_null(query1) - sum(query1)', agg: 'last', bad: true }),
          qv('Peak ingress (bit/s)', max(d('connection_bps_ingress'))),
          qv('Peak egress (bit/s)', max(d('connection_bps_egress'))),
          qv('Errors', sum(d('connection_error_count')), { bad: true }),
          qv('BGP sessions down', min(d('virtual_interface_bgp_status'), v), { formula: 'count_not_null(query1) - sum(query1)', agg: 'last', bad: true }),
        ],
        groups: [
          group('Connection state and errors', [
            ts('Connection state', min(d('connection_state'), c), { w: 4 }),
            ts('CRC and other errors', [sum(d('connection_crcerror_count')), sum(d('connection_error_count'))], { alias: ['CRC', 'all'], palette: 'warm', display: 'bars', w: 4 }),
            ts('Light levels (dBm)', [avg(d('connection_light_level_rx')), avg(d('connection_light_level_tx'))], { alias: ['received', 'sent'], w: 4 }),
          ]),
          group('Connection traffic', [
            ts('Bits per second both ways', [avg(d('connection_bps_ingress')), avg(d('connection_bps_egress'))], {
              formulas: [
                ['query1', 'ingress'],
                ['0 - query2', 'egress'],
              ],
            }),
            ts('Packets per second both ways', [avg(d('connection_pps_ingress')), avg(d('connection_pps_egress'))], { alias: ['ingress', 'egress'] }),
            top('Busiest connections', max(d('connection_bps_egress'), c)),
            ts('Egress discards', avg(d('connection_discards_pps_egress'), c), { palette: 'warm' }),
          ]),
          group('Virtual interfaces and BGP', [
            ts('Interface bits per second', avg(d('virtual_interface_bps_egress'), v), { w: 4 }),
            ts('BGP status by interface', min(d('virtual_interface_bgp_status'), v), { w: 4 }),
            tbl(
              'Interfaces',
              [
                ['Prefixes accepted', avg(d('virtual_interface_bgp_prefixes_accepted'), v), 'last'],
                ['Prefixes advertised', avg(d('virtual_interface_bgp_prefixes_advertised'), v), 'last'],
                ['Ingress bit/s', avg(d('virtual_interface_bps_ingress'), v)],
              ],
              { w: 4, h: 2 },
            ),
          ]),
        ],
      };
    },
  },

  'AWS/AppSync': {
    vars: ['graphqlapiid'],
    monitor: 'amazon_appsync',
    note: 'AppSync GraphQL APIs: client and server errors and the time each request takes, per API.',
    units: { Latency: 'millisecond' },
    build: ({ sum, avg, max }) => {
      const a = 'graphqlapiid';
      const s = (m) => `aws.appsync.${m}`;
      return {
        kpis: [
          qv('Client errors (4xx)', sum(s('4xxerror')), { bad: true }),
          qv('Server errors (5xx)', sum(s('5xxerror')), { bad: true }),
          qv('Avg latency', avg(s('latency'))),
          qv('Slowest request', max(s('latency.maximum'))),
        ],
        groups: [
          group('GraphQL errors', [
            ts('Errors by class', [sum(s('4xxerror')), sum(s('5xxerror'))], { alias: ['4xx', '5xx'], palette: 'warm', display: 'bars' }),
            top('APIs with most server errors', sum(s('5xxerror'), a), { palette: 'warm' }),
          ]),
          group('GraphQL latency', [ts('Latency by API', avg(s('latency'), a), { w: 8 }), top('Slowest APIs', avg(s('latency'), a), { w: 4 })]),
        ],
      };
    },
  },

  'AWS/CodeBuild': {
    vars: ['projectname'],
    monitor: 'amazon_codebuild',
    note: 'CodeBuild projects: builds started, succeeded and failed, and how long builds take. Each build’s phases and logs are in CI Visibility when the project sends them.',
    units: { Duration: 'second' },
    logs: [['CodeBuild logs', 'source:codebuild']],
    build: ({ sum, avg, max }) => {
      const p = 'projectname';
      const b = (m) => `aws.codebuild.${m}`;
      return {
        kpis: [
          qv('Builds', sum(b('builds'))),
          qv('Succeeded', sum(b('succeeded_builds')), { good: true }),
          qv('Failed', sum(b('failed_builds')), { bad: true }),
          qv('Failure rate', [sum(b('failed_builds')), sum(b('builds'))], { formula: '100 * query1 / query2', precision: 1, unit: '%' }),
          qv('Avg duration', avg(b('duration'))),
        ],
        groups: [
          group('Build outcomes', [
            ts('Succeeded and failed builds', [sum(b('succeeded_builds')), sum(b('failed_builds'))], { alias: ['succeeded', 'failed'], display: 'bars' }),
            top('Projects failing most', sum(b('failed_builds'), p), { palette: 'warm' }),
            tree('Builds by project', sum(b('builds'), p)),
            ts('Failure rate by project', [sum(b('failed_builds'), p), sum(b('builds'), p)], { formulas: [['100 * query1 / query2', '%']], palette: 'warm' }),
          ]),
          group('Build time', [ts('Duration by project', avg(b('duration'), p), { w: 8 }), top('Longest builds', max(b('duration.maximum'), p), { w: 4 })]),
        ],
      };
    },
  },

  'AWS/StorageGateway': {
    vars: ['gatewayid', 'gatewayname'],
    monitor: 'amazon_storage_gateway',
    note: 'Storage Gateway appliances: the local cache (hits, used, dirty data not yet uploaded), the upload buffer, reads and writes with their latency, traffic to and from AWS, and the gateway host’s CPU, I/O wait and memory.',
    units: {
      CacheHitPercent: 'percent',
      CachePercentDirty: 'percent',
      CachePercentUsed: 'percent',
      CacheUsed: 'byte',
      TotalCacheSize: 'byte',
      UploadBufferPercentUsed: 'percent',
      UploadBufferUsed: 'byte',
      ReadBytes: 'byte',
      WriteBytes: 'byte',
      ReadTime: 'millisecond',
      WriteTime: 'millisecond',
      CloudBytesDownloaded: 'byte',
      CloudBytesUploaded: 'byte',
      IoWaitPercent: 'percent',
      UserCpuPercent: 'percent',
      MemTotalBytes: 'byte',
      MemUsedBytes: 'byte',
      TimeSinceLastRecoveryPoint: 'second',
    },
    build: ({ sum, avg, max, min }) => {
      const g = 'gatewayname';
      const s = (m) => `aws.storagegateway.${m}`;
      return {
        kpis: [
          qv('Lowest cache hit %', min(s('cache_hit_percent')), { precision: 1 }),
          qv('Most dirty cache %', max(s('cache_percent_dirty')), { precision: 1 }),
          qv('Fullest upload buffer %', max(s('upload_buffer_percent_used')), { precision: 1 }),
          qv('Uploaded to AWS', sum(s('cloud_bytes_uploaded'))),
          qv('Health notifications', sum(s('health_notification')), { bad: true }),
        ],
        groups: [
          group('Local cache', [
            ts('Cache hits by gateway', avg(s('cache_hit_percent'), g), { w: 4 }),
            ts('Dirty cache by gateway', max(s('cache_percent_dirty'), g), { w: 4 }),
            ts('Cache used and its size', [sum(s('cache_used')), sum(s('total_cache_size'))], { alias: ['used', 'size'], w: 4 }),
          ]),
          group('Uploads to AWS', [
            ts('Upload buffer used', max(s('upload_buffer_percent_used'), g)),
            ts('Bytes to and from AWS', [sum(s('cloud_bytes_uploaded')), sum(s('cloud_bytes_downloaded'))], { alias: ['uploaded', 'downloaded'], display: 'bars' }),
            ts('Queued writes', max(s('queued_writes'), g)),
            top('Longest since a recovery point', max(s('time_since_last_recovery_point'), 'volumeid')),
          ]),
          group('Application I/O', [
            ts('Bytes read and written', [sum(s('read_bytes')), sum(s('write_bytes'))], { alias: ['read', 'written'] }),
            ts('Read and write time', [avg(s('read_time')), avg(s('write_time'))], { alias: ['read', 'write'] }),
          ]),
          group('Gateway host', [
            ts('CPU and I/O wait', [avg(s('user_cpu_percent')), avg(s('io_wait_percent'))], { alias: ['CPU', 'I/O wait'], w: 4 }),
            top('Highest memory used', [max(s('mem_used_bytes'), g), max(s('mem_total_bytes'), g)], { formula: '100 * query1 / query2', w: 4 }),
            ts('Availability and health notifications', [sum(s('availability_notifications')), sum(s('health_notification'))], {
              alias: ['availability', 'health'],
              palette: 'warm',
              display: 'bars',
              w: 4,
            }),
          ]),
        ],
      };
    },
  },

  'AWS/FSx': {
    vars: ['filesystemid'],
    monitor: 'amazon_fsx',
    note: 'FSx file systems (Windows File Server, Lustre, ONTAP, OpenZFS): data and metadata operations, throughput, free storage, the file server’s CPU and memory, disk throughput and IOPS against their burst balance, and client connections.',
    units: {
      DataReadBytes: 'byte',
      DataWriteBytes: 'byte',
      FreeStorageCapacity: 'byte',
      FreeDataStorageCapacity: 'byte',
      CPUUtilization: 'percent',
      MemoryUtilization: 'percent',
      NetworkThroughputUtilization: 'percent',
      DiskThroughputUtilization: 'percent',
      DiskThroughputBalance: 'percent',
      FileServerDiskIopsUtilization: 'percent',
      FileServerDiskIopsBalance: 'percent',
      FileServerDiskThroughputUtilization: 'percent',
    },
    build: ({ sum, avg, max, min }) => {
      const fs = 'filesystemid';
      const x = (m) => `aws.fsx.${m}`;
      return {
        kpis: [
          qv('Bytes read', sum(x('data_read_bytes'))),
          qv('Bytes written', sum(x('data_write_bytes'))),
          qv('Least free storage', min(x('free_storage_capacity'))),
          qv('Busiest file server CPU', max(x('cpuutilization')), { precision: 1 }),
          qv('Client connections', sum(x('client_connections')), { agg: 'last' }),
        ],
        groups: [
          group('File operations', [
            ts('Throughput', [sum(x('data_read_bytes')), sum(x('data_write_bytes'))], { alias: ['read', 'written'], display: 'bars', w: 4 }),
            ts('Operations', [sum(x('data_read_operations')), sum(x('data_write_operations')), sum(x('metadata_operations'))], { alias: ['reads', 'writes', 'metadata'], w: 4 }),
            top('Busiest file systems', sum(x('data_read_bytes'), fs), { w: 4 }),
          ]),
          group('Storage left', [
            top('Least free storage', min(x('free_storage_capacity'), fs), { order: 'asc', agg: 'last' }),
            chg('Free storage against the week before', min(x('free_storage_capacity'), fs), { increaseGood: true }),
          ]),
          group('File server and disks', [
            ts('CPU and memory', [avg(x('cpuutilization')), avg(x('memory_utilization'))], { alias: ['CPU', 'memory'], w: 4 }),
            ts('Network throughput used', max(x('network_throughput_utilization'), fs), { w: 4 }),
            ts('Disk burst balances', [min(x('disk_throughput_balance')), min(x('file_server_disk_iops_balance'))], { alias: ['throughput', 'IOPS'], w: 4 }),
            ts('Disk throughput and IOPS used', [max(x('disk_throughput_utilization')), max(x('file_server_disk_iops_utilization'))], { alias: ['throughput', 'IOPS'], w: 12 }),
          ]),
        ],
      };
    },
  },

  'AWS/TrustedAdvisor': {
    vars: ['checkname'],
    monitor: 'amazon_trusted_advisor',
    note: 'Trusted Advisor (Business or Enterprise support): the resources each check flags red (action recommended) or yellow (investigate), and how close each service limit is to its quota. CloudWatch reports them from us-east-1.',
    units: { ServiceLimitUsage: 'percent' },
    build: ({ max }) => {
      const c = 'checkname';
      const t = (m) => `aws.trustedadvisor.${m}`;
      return {
        kpis: [
          qv('Red resources', max(t('red_resources'), c), { formula: 'sum(query1)', agg: 'last', bad: true, span: '1d' }),
          qv('Yellow resources', max(t('yellow_resources'), c), { formula: 'sum(query1)', agg: 'last', span: '1d' }),
          qv('Highest limit usage', max(t('service_limit_usage')), { agg: 'last', precision: 1, span: '1d' }),
        ],
        groups: [
          group('Flagged checks', [
            top('Checks with red resources', max(t('red_resources'), c), { agg: 'last', palette: 'warm', span: '1d' }),
            top('Checks with yellow resources', max(t('yellow_resources'), c), { agg: 'last', span: '1d' }),
            ts('Flagged resources over the week', [max(t('red_resources')), max(t('yellow_resources'))], { alias: ['red', 'yellow'], span: '1w', w: 12 }),
          ]),
          group('Service limits', [
            top('Closest to their limit', max(t('service_limit_usage'), 'servicename,servicelimit'), { agg: 'last', span: '1d', w: 8 }),
            txt('Usage is a percentage of the limit. Limits above 80% are worth raising before they block a deployment.', { w: 4 }),
          ]),
        ],
      };
    },
  },

  'AWS/Usage': {
    vars: ['service', 'type', 'resource'],
    monitor: 'amazon_service_quotas',
    note: 'Usage that Service Quotas tracks: API calls per service and operation, resources in use (vCPUs, IPs …) and calls throttled for hitting a rate quota. Usage against the quota itself is the SERVICE_QUOTA() of a metric in CloudWatch.',
    build: ({ sum, max }) => {
      const u = (m) => `aws.usage.${m}`;
      return {
        kpis: [
          qv('API calls', sum(u('call_count'))),
          qv('Throttled calls', sum(u('throttle_count')), { bad: true }),
          qv('Throttle rate', [sum(u('throttle_count')), sum(u('call_count'))], { formula: '100 * query1 / query2', precision: 2, unit: '%' }),
          qv('Resource types counted', max(u('resource_count'), 'service,resource'), { formula: 'count_not_null(query1)' }),
        ],
        groups: [
          group('API calls by service', [
            ts('Calls by service', sum(u('call_count'), 'service'), { display: 'bars' }),
            top('Most called operations', sum(u('call_count'), 'service,resource')),
            top('Most throttled operations', sum(u('throttle_count'), 'service,resource'), { palette: 'warm' }),
            ts('Throttled calls by service', sum(u('throttle_count'), 'service'), { palette: 'warm' }),
          ]),
          group('Resources in use', [
            tbl('Resources', [['In use', max(u('resource_count'), 'service,resource'), 'last']], { w: 8 }),
            chg('Against the week before', max(u('resource_count'), 'service,resource'), { increaseGood: false, w: 4, h: 3 }),
          ]),
        ],
      };
    },
  },

  'AWS/ElasticInference': {
    vars: ['elasticinferenceacceleratorid'],
    monitor: 'amazon_elastic_inference',
    note: 'Elastic Inference accelerators attached to instances: health and connectivity checks, utilization and memory, and inferences served with their client and server errors.',
    build: ({ sum, avg, max }) => {
      const a = 'elasticinferenceacceleratorid';
      const e = (m) => `aws.elasticinference.${m}`;
      return {
        kpis: [
          qv('Inferences', sum(e('accelerator_total_inference_count'))),
          qv('Server errors', sum(e('accelerator_inference_with_server_error_count')), { bad: true }),
          qv('Failed health checks', max(e('accelerator_health_check_failed')), { agg: 'last', bad: true }),
          qv('Avg utilization', avg(e('accelerator_utilization')), { precision: 1 }),
        ],
        groups: [
          group('Inferences served', [
            ts(
              'Successful and failed inferences',
              [sum(e('accelerator_successful_inference_count')), sum(e('accelerator_inference_with_client_error_count')), sum(e('accelerator_inference_with_server_error_count'))],
              {
                alias: ['succeeded', 'client error', 'server error'],
                display: 'bars',
              },
            ),
            top('Busiest accelerators', sum(e('accelerator_total_inference_count'), a)),
          ]),
          group('Accelerator health', [
            ts('Utilization and memory', [avg(e('accelerator_utilization')), avg(e('accelerator_memory_usage'))], { alias: ['utilization', 'memory'] }),
            ts('Failed checks', [max(e('accelerator_health_check_failed')), max(e('connectivity_check_failed'))], { alias: ['health', 'connectivity'], palette: 'warm' }),
          ]),
        ],
      };
    },
  },

  'AWS/ElasticTranscoder': {
    vars: ['pipelineid'],
    monitor: 'amazon_elastic_transcoder',
    note: 'Elastic Transcoder pipelines: jobs completed and failed, outputs per job, billed HD, SD and audio minutes, time jobs wait in standby, and API errors and throttles.',
    build: ({ sum, avg }) => {
      const p = 'pipelineid';
      const t = (m) => `aws.elastictranscoder.${m}`;
      return {
        kpis: [
          qv('Jobs completed', sum(t('jobs_completed')), { good: true }),
          qv('Jobs failed', sum(t('jobs_errored')), { bad: true }),
          qv('Billed HD minutes', sum(t('billed_hdoutput'))),
          qv('Throttled calls', sum(t('throttles')), { bad: true }),
        ],
        groups: [
          group('Transcoding jobs', [
            ts('Completed and failed jobs', [sum(t('jobs_completed')), sum(t('jobs_errored'))], { alias: ['completed', 'failed'], display: 'bars' }),
            top('Pipelines failing most', sum(t('jobs_errored'), p), { palette: 'warm' }),
            ts('Standby time by pipeline', avg(t('standby_time'), p)),
            ts('Outputs per job', avg(t('outputs_per_job'), p)),
          ]),
          group('Billed output', [
            ts('Billed minutes', [sum(t('billed_hdoutput')), sum(t('billed_sdoutput')), sum(t('billed_audio_output'))], { alias: ['HD', 'SD', 'audio'], display: 'area', w: 8 }),
            ts('API errors and throttles', [sum(t('errors')), sum(t('throttles'))], { alias: ['errors', 'throttles'], palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },
};
