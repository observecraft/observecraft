// The AWS, Azure and Google Cloud overview dashboards. Datadog's aren't open source (the cloud integrations
// live outside integrations-core), so these are ObserveCraft's own: the same kind of layout (a banner with the
// cloud's services, its monitors and logs, then a colored group per service with KPIs, top lists and trends),
// written in the Datadog widget format the integration dashboards use and converted like them
// (import-integration-dashboards.mjs). Metric names are the ones our crawlers publish: Datadog names, the
// CloudWatch average without suffix and `.sum` / `.maximum` (AWS), Azure's `.total` / `.maximum`.
// Artwork: web/scripts/build-cloud-artwork.mjs.
// node scripts/cloud-dashboards.mjs   (writes server/seeds/integrations/{aws,azure,gcp}/dashboards.json)
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { convertDashboard } from './import-integration-dashboards.mjs';

export const CLOUDS = ['aws', 'azure', 'gcp'];

// ---------------------------------------------------------------- widget builders (Datadog format)

export const metric = (query, aggregator = 'avg', name = 'query1') => ({ data_source: 'metrics', name, query, aggregator });

export function queryValue(title, queries, { formula = 'query1', precision = 0, unit, formats = [] } = {}) {
  return {
    type: 'query_value',
    title,
    title_size: '16',
    title_align: 'left',
    requests: [{ response_format: 'scalar', queries, formulas: [{ formula }], conditional_formats: formats }],
    autoscale: true,
    precision,
    ...(unit ? { custom_unit: unit } : {}),
  };
}

export function timeseries(title, queries, { formulas, display = 'line', palette = 'dog_classic' } = {}) {
  return {
    type: 'timeseries',
    title,
    title_size: '16',
    title_align: 'left',
    show_legend: true,
    legend_layout: 'auto',
    requests: [{ response_format: 'timeseries', queries, formulas: formulas ?? queries.map((q) => ({ formula: q.name })), style: { palette, order_by: 'values' }, display_type: display }],
    yaxis: { include_zero: true },
  };
}

function toplist(title, query, { aggregator = 'avg', count = 10, formula = 'query1', palette, order = 'desc' } = {}) {
  return {
    type: 'toplist',
    title,
    title_size: '16',
    title_align: 'left',
    requests: [
      {
        response_format: 'scalar',
        queries: [metric(query, aggregator)],
        formulas: [{ formula }],
        sort: { count, order_by: [{ type: 'formula', index: 0, order }] },
        ...(palette ? { style: { palette } } : {}),
      },
    ],
  };
}

/** Columns: [alias, query, aggregator]; rows: the first query's groups. */
export function table(title, columns, count = 50) {
  const queries = columns.map(([, query, aggregator], i) => metric(query, aggregator ?? 'avg', `query${i + 1}`));
  return {
    type: 'query_table',
    title,
    title_size: '16',
    title_align: 'left',
    requests: [
      { response_format: 'scalar', queries, formulas: columns.map(([alias], i) => ({ alias, formula: `query${i + 1}` })), sort: { count, order_by: [{ type: 'formula', index: 0, order: 'desc' }] } },
    ],
  };
}

export const treemap = (title, query, aggregator = 'avg') => ({
  type: 'treemap',
  title,
  requests: [{ response_format: 'scalar', queries: [metric(query, aggregator)], formulas: [{ formula: 'query1' }] }],
});

export const note = (content, background_color, { align = 'center', size = '14', tick = false } = {}) => ({
  type: 'note',
  content,
  background_color,
  font_size: size,
  text_align: align,
  show_tick: tick,
  tick_pos: '50%',
  tick_edge: 'left',
  has_padding: true,
});

const OK = [{ comparator: '>', value: 0, palette: 'green_on_white' }];
const BAD = [{ comparator: '>', value: 0, palette: 'red_on_white' }];

// ---------------------------------------------------------------- layout

/** A dashboard under construction: widgets with Datadog ordered-layout coordinates (12 columns). */
function builder(cloud) {
  let seq = 0;
  const overrides = new Map();
  const layouts = new Map();
  const place = (definition, x, y, width, height) => {
    const w = { id: ++seq, definition, layout: { x, y, width, height } };
    layouts.set(w.id, w.layout);
    return w;
  };
  return {
    place,
    /** Service groups come after the intro (y orders them). */
    group: (title, color, children, y, height) => place({ type: 'group', title, background_color: color, layout_type: 'ordered', widgets: children }, 0, 100 + y, 12, height),
    /** Widgets the converter doesn't know become our own definitions after conversion. */
    native(definition, x, y, width, height) {
      const w = place({ type: 'note', content: '' }, x, y, width, height);
      overrides.set(w.id, definition);
      return w;
    },
    tile: (id, label, x, y) => {
      const w = place({ type: 'note', content: '' }, x, y, 2, 2);
      overrides.set(w.id, { type: 'image', title: label, src: `/images/integrations/tiles/${cloud}-${id}.svg` });
      return w;
    },
    overrides,
    layouts,
  };
}

/** The intro: banner, text and service tiles on the left; monitors and logs on the right. */
function intro(b, cloud, text, tiles, monitors, logs) {
  return [
    b.place(
      {
        type: 'group',
        title: '',
        show_title: false,
        banner_img: `/images/integrations/${cloud}-banner.svg`,
        layout_type: 'ordered',
        widgets: [b.place(note(text, 'blue', { size: '14' }), 0, 0, 6, 2), ...tiles.map(([id, label], i) => b.tile(id, label, (i % 3) * 2, 2 + Math.floor(i / 3) * 2))],
      },
      0,
      0,
      6,
      9,
    ),
    b.native({ type: 'monitor_summary', title: monitors.title, query: monitors.query }, 6, 0, 6, 5),
    b.native({ type: 'log_stream', title: logs.title, query: logs.query, limit: 50 }, 6, 5, 6, 4),
  ];
}

// ---------------------------------------------------------------- AWS

function aws() {
  const b = builder('aws');
  const f = '$account,$region';
  const section = (text) => note(text, 'yellow');
  const widgets = [
    ...intro(
      b,
      'aws',
      'A starting point for the AWS services ObserveCraft collects from CloudWatch. Add accounts, regions and services in [Integrations → Amazon Web Services](/integrations/aws); every chart filters by account and region.',
      [
        ['ec2', 'Amazon EC2'],
        ['rds', 'Amazon RDS'],
        ['elb', 'Elastic Load Balancing'],
        ['lambda', 'AWS Lambda'],
        ['s3', 'Amazon S3'],
        ['ebs', 'Amazon EBS'],
      ],
      { title: 'AWS monitors', query: 'aws' },
      { title: 'AWS logs', query: 'source:(cloudwatch OR lambda OR elb OR s3 OR rds OR cloudtrail OR vpc)' },
    ),
    b.group(
      'EC2',
      'vivid_orange',
      [
        b.place(section('Instances, their CPU and network. Consistently high CPU is normal for some workloads; maxed-out CPU or failed status checks need a look.'), 0, 0, 12, 1),
        b.place(queryValue('Active EC2 instances', [metric(`avg:aws.ec2.cpuutilization{${f}} by {instanceid}`)], { formula: 'count_nonzero(query1)', formats: OK }), 0, 1, 2, 1),
        b.place(queryValue('Failed status checks', [metric(`sum:aws.ec2.status_check_failed.maximum{${f}}`, 'max')], { formats: BAD }), 2, 1, 2, 1),
        b.place(queryValue('Total network in', [metric(`sum:aws.ec2.network_in.sum{${f}}`, 'sum')]), 4, 1, 2, 1),
        b.place(queryValue('Total network out', [metric(`sum:aws.ec2.network_out.sum{${f}}`, 'sum')]), 6, 1, 2, 1),
        b.place(queryValue('Avg CPU utilization', [metric(`avg:aws.ec2.cpuutilization{${f}}`)], { precision: 1 }), 8, 1, 2, 1),
        b.place(queryValue('Max CPU utilization', [metric(`max:aws.ec2.cpuutilization.maximum{${f}}`, 'max')], { precision: 1 }), 10, 1, 2, 1),
        b.place(timeseries('CPU utilization by region', [metric(`avg:aws.ec2.cpuutilization{${f}} by {region}`)], { palette: 'orange' }), 0, 2, 4, 4),
        b.place(
          timeseries('Network I/O', [metric(`sum:aws.ec2.network_in.sum{${f}}`, 'sum', 'query1'), metric(`sum:aws.ec2.network_out.sum{${f}}`, 'sum', 'query2')], {
            formulas: [
              { formula: 'query1', alias: 'in' },
              { formula: '0 - query2', alias: 'out' },
            ],
            display: 'bars',
            palette: 'warm',
          }),
          4,
          2,
          4,
          4,
        ),
        b.place(
          table('CPU utilization (top 50)', [
            ['CPU %', `avg:aws.ec2.cpuutilization{${f}} by {instanceid,region}`],
            ['Max CPU %', `max:aws.ec2.cpuutilization.maximum{${f}} by {instanceid,region}`, 'max'],
          ]),
          8,
          2,
          4,
          4,
        ),
      ],
      1,
      7,
    ),
    b.group(
      'RDS',
      'vivid_orange',
      [
        b.place(section('Database instances: connections, CPU, storage and latency.'), 0, 0, 12, 1),
        b.place(queryValue('Total connections', [metric(`sum:aws.rds.database_connections{${f},dbinstanceidentifier:*}`)]), 0, 1, 3, 1),
        b.place(queryValue('Avg CPU utilization', [metric(`avg:aws.rds.cpuutilization{${f},dbinstanceidentifier:*}`)], { precision: 1 }), 3, 1, 3, 1),
        b.place(queryValue('Total free storage', [metric(`sum:aws.rds.free_storage_space{${f},dbinstanceidentifier:*}`)]), 6, 1, 3, 1),
        b.place(queryValue('Read IOPS', [metric(`sum:aws.rds.read_iops{${f},dbinstanceidentifier:*}`)], { precision: 1 }), 9, 1, 3, 1),
        b.place(toplist('Highest read latency', `avg:aws.rds.read_latency{${f},dbinstanceidentifier:*} by {dbinstanceidentifier}`, { palette: 'orange' }), 0, 2, 6, 3),
        b.place(toplist('Least free storage', `avg:aws.rds.free_storage_space{${f},dbinstanceidentifier:*} by {dbinstanceidentifier}`, { order: 'asc' }), 6, 2, 6, 3),
        b.place(timeseries('Connections by database', [metric(`avg:aws.rds.database_connections{${f},dbinstanceidentifier:*} by {dbinstanceidentifier}`)]), 0, 5, 6, 3),
        b.place(treemap('Write latency by database', `avg:aws.rds.write_latency{${f},dbinstanceidentifier:*} by {dbinstanceidentifier}`), 6, 5, 6, 3),
      ],
      2,
      9,
    ),
    b.group(
      'Load Balancers',
      'vivid_orange',
      [
        b.place(section('Classic Load Balancers (ELB) and Application Load Balancers (ALB): healthy hosts, traffic, errors and response time.'), 0, 0, 12, 1),
        b.place(queryValue('Healthy ELB hosts', [metric(`sum:aws.elb.healthy_host_count{${f}}`)], { formats: OK }), 0, 1, 2, 1),
        b.place(queryValue('Unhealthy ELB hosts', [metric(`sum:aws.elb.un_healthy_host_count{${f}}`)], { formats: BAD }), 2, 1, 2, 1),
        b.place(queryValue('Healthy ALB targets', [metric(`sum:aws.applicationelb.healthy_host_count{${f}}`)], { formats: OK }), 6, 1, 2, 1),
        b.place(queryValue('Unhealthy ALB targets', [metric(`sum:aws.applicationelb.un_healthy_host_count{${f}}`)], { formats: BAD }), 8, 1, 2, 1),
        b.place(queryValue('ELB 5xx errors', [metric(`sum:aws.elb.httpcode_elb_5xx.sum{${f}}`, 'sum')], { formats: BAD }), 4, 1, 2, 1),
        b.place(queryValue('ALB 5xx errors', [metric(`sum:aws.applicationelb.httpcode_elb_5xx_count.sum{${f}}`, 'sum')], { formats: BAD }), 10, 1, 2, 1),
        b.place(timeseries('ELB requests', [metric(`sum:aws.elb.request_count.sum{${f}} by {loadbalancername}`, 'sum')], { display: 'bars' }), 0, 2, 6, 3),
        b.place(timeseries('ALB requests', [metric(`sum:aws.applicationelb.request_count.sum{${f}} by {loadbalancer}`, 'sum')], { display: 'bars' }), 6, 2, 6, 3),
        b.place(timeseries('ELB latency', [metric(`avg:aws.elb.latency{${f}} by {loadbalancername}`)]), 0, 5, 6, 2),
        b.place(timeseries('ALB target response time', [metric(`avg:aws.applicationelb.target_response_time{${f}} by {loadbalancer}`)]), 6, 5, 6, 2),
      ],
      3,
      8,
    ),
    b.group(
      'Lambda',
      'vivid_orange',
      [
        b.place(section('Functions: invocations, errors, duration and throttles.'), 0, 0, 12, 1),
        b.place(queryValue('Invocations', [metric(`sum:aws.lambda.invocations.sum{${f}}`, 'sum')]), 0, 1, 2, 1),
        b.place(queryValue('Errors', [metric(`sum:aws.lambda.errors.sum{${f}}`, 'sum')], { formats: BAD }), 2, 1, 2, 1),
        b.place(
          queryValue('Error rate', [metric(`sum:aws.lambda.errors.sum{${f}}`, 'sum', 'query1'), metric(`sum:aws.lambda.invocations.sum{${f}}`, 'sum', 'query2')], {
            formula: '100 * query1 / query2',
            precision: 2,
            unit: '%',
          }),
          4,
          1,
          2,
          1,
        ),
        b.place(queryValue('Avg duration', [metric(`avg:aws.lambda.duration{${f}}`)], { precision: 0 }), 6, 1, 2, 1),
        b.place(queryValue('Throttles', [metric(`sum:aws.lambda.throttles.sum{${f}}`, 'sum')], { formats: BAD }), 8, 1, 2, 1),
        b.place(queryValue('Peak concurrency', [metric(`max:aws.lambda.concurrent_executions.maximum{${f}}`, 'max')]), 10, 1, 2, 1),
        b.place(
          table('Functions', [
            ['Invocations', `sum:aws.lambda.invocations.sum{${f}} by {functionname}`, 'sum'],
            ['Errors', `sum:aws.lambda.errors.sum{${f}} by {functionname}`, 'sum'],
            ['Avg duration (ms)', `avg:aws.lambda.duration{${f}} by {functionname}`],
          ]),
          0,
          2,
          8,
          5,
        ),
        b.place(timeseries('Errors by function', [metric(`sum:aws.lambda.errors.sum{${f}} by {functionname}`, 'sum')], { display: 'bars', palette: 'warm' }), 8, 2, 4, 3),
        b.place(timeseries('Throttles', [metric(`sum:aws.lambda.throttles.sum{${f}} by {functionname}`, 'sum')], { display: 'bars', palette: 'warm' }), 8, 5, 4, 2),
      ],
      4,
      8,
    ),
    b.group(
      'S3',
      'vivid_orange',
      [
        b.place(section('Buckets: size and objects (daily storage metrics) and requests (with request metrics enabled on the bucket).'), 0, 0, 12, 1),
        b.place(queryValue('Storage', [metric(`sum:aws.s3.bucket_size_bytes{${f}}`)]), 0, 1, 3, 1),
        b.place(queryValue('Objects', [metric(`sum:aws.s3.number_of_objects{${f}}`)]), 3, 1, 3, 1),
        b.place(queryValue('Requests', [metric(`sum:aws.s3.all_requests.sum{${f}}`, 'sum')]), 6, 1, 3, 1),
        b.place(queryValue('4xx errors', [metric(`sum:aws.s3.4xx_errors.sum{${f}}`, 'sum')], { formats: BAD }), 9, 1, 3, 1),
        b.place(toplist('Storage by bucket', `avg:aws.s3.bucket_size_bytes{${f}} by {bucketname}`), 0, 2, 6, 3),
        b.place(treemap('Objects by bucket', `avg:aws.s3.number_of_objects{${f}} by {bucketname}`), 6, 2, 6, 3),
      ],
      5,
      6,
    ),
    b.group(
      'EBS',
      'vivid_orange',
      [
        b.place(section('Volumes: operations, throughput, queue length and burst credits.'), 0, 0, 12, 1),
        b.place(queryValue('Volumes', [metric(`avg:aws.ebs.volume_queue_length{${f}} by {volumeid}`)], { formula: 'count_nonzero(query1)' }), 0, 1, 3, 1),
        b.place(queryValue('Avg burst balance', [metric(`avg:aws.ebs.burst_balance{${f}}`)], { precision: 1 }), 3, 1, 3, 1),
        b.place(queryValue('Avg queue length', [metric(`avg:aws.ebs.volume_queue_length{${f}}`)], { precision: 2 }), 6, 1, 3, 1),
        b.place(queryValue('Idle time', [metric(`avg:aws.ebs.volume_idle_time.sum{${f}}`, 'avg')]), 9, 1, 3, 1),
        b.place(
          timeseries('Operations', [metric(`sum:aws.ebs.volume_read_ops.sum{${f}}`, 'sum', 'query1'), metric(`sum:aws.ebs.volume_write_ops.sum{${f}}`, 'sum', 'query2')], {
            formulas: [
              { formula: 'query1', alias: 'reads' },
              { formula: 'query2', alias: 'writes' },
            ],
          }),
          0,
          2,
          6,
          3,
        ),
        b.place(
          timeseries('Throughput', [metric(`sum:aws.ebs.volume_read_bytes.sum{${f}}`, 'sum', 'query1'), metric(`sum:aws.ebs.volume_write_bytes.sum{${f}}`, 'sum', 'query2')], {
            formulas: [
              { formula: 'query1', alias: 'read' },
              { formula: 'query2', alias: 'written' },
            ],
          }),
          6,
          2,
          6,
          3,
        ),
        b.place(toplist('Longest queues', `avg:aws.ebs.volume_queue_length{${f}} by {volumeid}`, { palette: 'orange' }), 0, 5, 12, 2),
      ],
      6,
      7,
    ),
  ];
  return {
    builder: b,
    doc: {
      title: 'AWS Overview',
      description: 'The AWS services ObserveCraft collects from CloudWatch: EC2, RDS, load balancers, Lambda, S3 and EBS.',
      layout_type: 'ordered',
      template_variables: [
        { name: 'account', prefix: 'aws_account', default: '*' },
        { name: 'region', prefix: 'region', default: '*' },
      ],
      widgets,
    },
  };
}

// ---------------------------------------------------------------- Azure

function azure() {
  const b = builder('azure');
  const f = '$subscription,$resource_group,$region';
  const section = (text) => note(text, 'vivid_purple');
  const tip = (text) => note(text, 'blue', { align: 'left', size: '14' });
  const widgets = [
    ...intro(
      b,
      'azure',
      'A starting point for the Azure services ObserveCraft collects from Azure Monitor. Add app registrations in [Integrations → Microsoft Azure](/integrations/azure); every chart filters by subscription, resource group and region.',
      [
        ['vm', 'Virtual Machines'],
        ['sql', 'SQL Database'],
        ['app-service', 'App Service'],
        ['functions', 'Functions'],
        ['lb', 'Load Balancer'],
        ['storage', 'Storage'],
      ],
      { title: 'Azure monitors', query: 'azure' },
      { title: 'Azure logs', query: 'source:azure*' },
    ),
    b.group(
      'Virtual Machines',
      'vivid_blue',
      [
        b.place(section('Virtual machines: CPU, memory, disks and network.'), 0, 0, 12, 1),
        b.place(queryValue('Running VMs', [metric(`sum:azure.vm.count{${f},status:running}`)], { formats: OK }), 0, 1, 2, 1),
        b.place(queryValue('Total network in', [metric(`sum:azure.vm.network_in_total{${f}}`, 'sum')]), 2, 1, 2, 1),
        b.place(queryValue('Total network out', [metric(`sum:azure.vm.network_out_total{${f}}`, 'sum')]), 4, 1, 2, 1),
        b.place(queryValue('Avg CPU utilization', [metric(`avg:azure.vm.percentage_cpu{${f}}`)], { precision: 1 }), 6, 1, 2, 1),
        b.place(queryValue('Max CPU utilization', [metric(`max:azure.vm.percentage_cpu.max{${f}}`, 'max')], { precision: 1 }), 8, 1, 2, 1),
        b.place(tip('High CPU can be normal; CPU pinned at its maximum slows the VM.'), 10, 1, 2, 1),
        b.place(
          timeseries('Network traffic', [metric(`sum:azure.vm.network_in_total{${f}}`, 'sum', 'query1'), metric(`sum:azure.vm.network_out_total{${f}}`, 'sum', 'query2')], {
            formulas: [
              { formula: 'query1', alias: 'in' },
              { formula: '0 - query2', alias: 'out' },
            ],
            display: 'bars',
            palette: 'cool',
          }),
          0,
          2,
          4,
          3,
        ),
        b.place(timeseries('Available memory', [metric(`avg:azure.vm.available_memory_bytes{${f}} by {name}`)], { palette: 'cool' }), 4, 2, 4, 3),
        b.place(
          table('CPU utilization (top 50)', [
            ['CPU %', `avg:azure.vm.percentage_cpu{${f}} by {name,resource_group}`],
            ['Max CPU %', `max:azure.vm.percentage_cpu.max{${f}} by {name,resource_group}`, 'max'],
          ]),
          8,
          2,
          4,
          3,
        ),
        b.place(
          timeseries('Disk throughput', [metric(`sum:azure.vm.disk_read_bytes{${f}}`, 'sum', 'query1'), metric(`sum:azure.vm.disk_write_bytes{${f}}`, 'sum', 'query2')], {
            formulas: [
              { formula: 'query1', alias: 'read' },
              { formula: 'query2', alias: 'written' },
            ],
            palette: 'cool',
          }),
          0,
          5,
          12,
          2,
        ),
      ],
      1,
      8,
    ),
    b.group(
      'SQL Databases',
      'vivid_blue',
      [
        b.place(section('SQL databases: connections, CPU, DTU and storage.'), 0, 0, 12, 1),
        b.place(queryValue('Failed connections', [metric(`sum:azure.sql_servers_databases.connection_failed{${f}}`, 'sum')], { formats: BAD }), 0, 1, 2, 1),
        b.place(queryValue('Successful connections', [metric(`sum:azure.sql_servers_databases.connection_successful{${f}}`, 'sum')], { formats: OK }), 2, 1, 2, 1),
        b.place(queryValue('Avg CPU', [metric(`avg:azure.sql_servers_databases.cpu_percent{${f}}`)], { precision: 1 }), 4, 1, 2, 1),
        b.place(queryValue('Deadlocks', [metric(`sum:azure.sql_servers_databases.deadlock{${f}}`, 'sum')], { formats: BAD }), 6, 1, 2, 1),
        b.place(tip('DTU consumption near 100% means the database needs a larger tier or lighter queries.'), 8, 1, 4, 1),
        b.place(treemap('Storage used by database', `avg:azure.sql_servers_databases.storage_percent{${f}} by {name}`), 0, 2, 8, 3),
        b.place(timeseries('DTU consumption by database', [metric(`avg:azure.sql_servers_databases.dtu_consumption_percent{${f}} by {name}`)], { palette: 'cool' }), 8, 2, 4, 3),
      ],
      2,
      6,
    ),
    b.group(
      'App Services',
      'vivid_blue',
      [
        b.place(section('Web apps and function apps: requests, errors, response time and executions.'), 0, 0, 12, 1),
        b.place(queryValue('Requests', [metric(`sum:azure.app_services.requests{${f}}`, 'sum')]), 0, 1, 2, 1),
        b.place(queryValue('5xx errors', [metric(`sum:azure.app_services.http5xx{${f}}`, 'sum')], { formats: BAD }), 0, 2, 2, 1),
        b.place(queryValue('Function executions', [metric(`sum:azure.app_services.function_execution_count{${f}}`, 'sum')]), 0, 3, 2, 1),
        b.place(
          table('Apps', [
            ['Requests', `sum:azure.app_services.requests{${f}} by {name}`, 'sum'],
            ['5xx', `sum:azure.app_services.http5xx{${f}} by {name}`, 'sum'],
            ['Response time (s)', `avg:azure.app_services.http_response_time{${f}} by {name}`],
          ]),
          2,
          1,
          10,
          3,
        ),
      ],
      3,
      5,
    ),
    b.group(
      'Load Balancers',
      'vivid_blue',
      [
        b.place(section('Load balancers: data path and health probe availability, traffic.'), 0, 0, 12, 1),
        b.place(queryValue('Data path availability', [metric(`avg:azure.network_loadbalancers.vip_availability{${f}}`)], { precision: 1 }), 0, 1, 2, 1),
        b.place(queryValue('Health probe status', [metric(`avg:azure.network_loadbalancers.dip_availability{${f}}`)], { precision: 1 }), 2, 1, 2, 1),
        b.place(tip('Data path availability below 100% means the frontend IP isn’t reachable; health probe status, that backends fail their probes.'), 4, 1, 8, 1),
        b.place(timeseries('Bytes by load balancer', [metric(`sum:azure.network_loadbalancers.byte_count{${f}} by {name}`, 'sum')], { display: 'bars', palette: 'cool' }), 0, 2, 4, 3),
        b.place(timeseries('Health probe status', [metric(`avg:azure.network_loadbalancers.dip_availability{${f}} by {name}`)], { palette: 'cool' }), 4, 2, 4, 3),
        b.place(timeseries('Data path availability', [metric(`avg:azure.network_loadbalancers.vip_availability{${f}} by {name}`)], { palette: 'cool' }), 8, 2, 4, 3),
      ],
      4,
      6,
    ),
    b.group(
      'Storage',
      'vivid_blue',
      [
        b.place(section('Storage accounts: availability, capacity, transactions and latency.'), 0, 0, 12, 1),
        b.place(queryValue('Availability', [metric(`avg:azure.storage.availability{${f}}`)], { precision: 2 }), 0, 1, 2, 1),
        b.place(queryValue('Used capacity', [metric(`sum:azure.storage.used_capacity{${f}}`)]), 2, 1, 2, 1),
        b.place(queryValue('Transactions', [metric(`sum:azure.storage.transactions{${f}}`, 'sum')]), 4, 1, 2, 1),
        b.place(timeseries('Used capacity by account', [metric(`avg:azure.storage.used_capacity{${f}} by {name}`)], { palette: 'cool' }), 6, 1, 6, 3),
        b.place(timeseries('Server latency', [metric(`avg:azure.storage.success_server_latency{${f}} by {name}`)], { palette: 'cool' }), 0, 2, 6, 2),
      ],
      5,
      5,
    ),
  ];
  return {
    builder: b,
    doc: {
      title: 'Azure Overview',
      description: 'The Azure services ObserveCraft collects from Azure Monitor: virtual machines, SQL databases, App Service, load balancers and storage.',
      layout_type: 'ordered',
      template_variables: [
        { name: 'subscription', prefix: 'subscription_id', default: '*' },
        { name: 'resource_group', prefix: 'resource_group', default: '*' },
        { name: 'region', prefix: 'region', default: '*' },
      ],
      widgets,
    },
  };
}

// ---------------------------------------------------------------- Google Cloud

function gcp() {
  const b = builder('gcp');
  const f = '$project';
  const section = (text, color) => note(text, color);
  const widgets = [
    ...intro(
      b,
      'gcp',
      'A starting point for the Google Cloud services ObserveCraft collects from Cloud Monitoring. Add service accounts in [Integrations → Google Cloud](/integrations/gcp); every chart filters by project.',
      [
        ['gce', 'Compute Engine'],
        ['cloudsql', 'Cloud SQL'],
        ['gke', 'Kubernetes Engine'],
        ['run', 'Cloud Run'],
        ['functions', 'Cloud Functions'],
        ['lb', 'Cloud Load Balancing'],
      ],
      { title: 'Google Cloud monitors', query: 'gcp' },
      { title: 'Google Cloud logs', query: 'source:gcp*' },
    ),
    b.group(
      'Compute Engine',
      'pink',
      [
        b.place(section('Instances: CPU, network and disks.', 'pink'), 0, 0, 12, 1),
        b.place(queryValue('Active instances', [metric(`avg:gcp.gce.instance.cpu.utilization{${f},$zone} by {instance_id}`)], { formula: 'count_nonzero(query1)', formats: OK }), 0, 1, 2, 1),
        b.place(queryValue('Total network in', [metric(`sum:gcp.gce.instance.network.received_bytes_count{${f},$zone}`, 'sum')]), 2, 1, 2, 1),
        b.place(queryValue('Total network out', [metric(`sum:gcp.gce.instance.network.sent_bytes_count{${f},$zone}`, 'sum')]), 4, 1, 2, 1),
        b.place(queryValue('Avg CPU utilization', [metric(`avg:gcp.gce.instance.cpu.utilization{${f},$zone}`)], { precision: 1 }), 6, 1, 2, 1),
        b.place(queryValue('Max CPU utilization', [metric(`max:gcp.gce.instance.cpu.utilization{${f},$zone}`, 'max')], { precision: 1 }), 8, 1, 2, 1),
        b.place(queryValue('Disk reads', [metric(`sum:gcp.gce.instance.disk.read_bytes_count{${f},$zone}`, 'sum')]), 10, 1, 2, 1),
        b.place(timeseries('CPU utilization by zone', [metric(`avg:gcp.gce.instance.cpu.utilization{${f},$zone} by {zone}`)], { palette: 'purple' }), 0, 2, 4, 4),
        b.place(
          timeseries(
            'Network I/O',
            [metric(`sum:gcp.gce.instance.network.received_bytes_count{${f},$zone}`, 'sum', 'query1'), metric(`sum:gcp.gce.instance.network.sent_bytes_count{${f},$zone}`, 'sum', 'query2')],
            {
              formulas: [
                { formula: 'query1', alias: 'in' },
                { formula: '0 - query2', alias: 'out' },
              ],
              display: 'bars',
              palette: 'purple',
            },
          ),
          4,
          2,
          4,
          4,
        ),
        b.place(table('CPU utilization (top 50)', [['CPU', `avg:gcp.gce.instance.cpu.utilization{${f},$zone} by {instance_id,zone}`]]), 8, 2, 4, 4),
      ],
      1,
      7,
    ),
    b.group(
      'Cloud SQL',
      'blue',
      [
        b.place(section('Database instances: connections, CPU, memory and disk.', 'blue'), 0, 0, 12, 1),
        b.place(queryValue('Connections', [metric(`sum:gcp.cloudsql.database.network.connections{${f},$region}`)]), 0, 1, 3, 1),
        b.place(queryValue('Avg CPU utilization', [metric(`avg:gcp.cloudsql.database.cpu.utilization{${f},$region}`)], { precision: 1 }), 3, 1, 3, 1),
        b.place(queryValue('Avg memory utilization', [metric(`avg:gcp.cloudsql.database.memory.utilization{${f},$region}`)], { precision: 1 }), 6, 1, 3, 1),
        b.place(queryValue('Disk used', [metric(`sum:gcp.cloudsql.database.disk.bytes_used{${f},$region}`)]), 9, 1, 3, 1),
        b.place(toplist('Fullest disks', `avg:gcp.cloudsql.database.disk.utilization{${f},$region} by {database_id}`, { palette: 'orange' }), 0, 2, 6, 3),
        b.place(timeseries('Connections by database', [metric(`avg:gcp.cloudsql.database.network.connections{${f},$region} by {database_id}`)], { palette: 'cool' }), 6, 2, 6, 3),
      ],
      2,
      6,
    ),
    b.group(
      'Kubernetes Engine',
      'green',
      [
        b.place(section('Clusters: node utilization of allocatable resources, container memory and restarts.', 'green'), 0, 0, 12, 1),
        b.place(timeseries('Node CPU allocatable utilization', [metric(`avg:gcp.gke.node.cpu.allocatable_utilization{${f}} by {cluster_name}`)], { palette: 'green' }), 0, 1, 6, 3),
        b.place(timeseries('Node memory allocatable utilization', [metric(`avg:gcp.gke.node.memory.allocatable_utilization{${f}} by {cluster_name}`)], { palette: 'green' }), 6, 1, 6, 3),
        b.place(toplist('Memory-intensive containers', `avg:gcp.gke.container.memory.used_bytes{${f}} by {cluster_name,namespace_name,container_name}`), 0, 4, 6, 3),
        b.place(toplist('Container restarts', `sum:gcp.gke.container.restart_count{${f}} by {cluster_name,namespace_name,container_name}`, { aggregator: 'sum', palette: 'orange' }), 6, 4, 6, 3),
      ],
      3,
      7,
    ),
    b.group(
      'Cloud Run and Cloud Functions',
      'purple',
      [
        b.place(section('Serverless: requests and instances of Cloud Run services, executions of functions.', 'purple'), 0, 0, 12, 1),
        b.place(queryValue('Cloud Run requests', [metric(`sum:gcp.run.request_count{${f}}`, 'sum')]), 0, 1, 3, 1),
        b.place(queryValue('Cloud Run 5xx', [metric(`sum:gcp.run.request_count{${f},response_code_class:5xx}`, 'sum')], { formats: BAD }), 3, 1, 3, 1),
        b.place(queryValue('Function executions', [metric(`sum:gcp.cloudfunctions.function.execution_count{${f}}`, 'sum')]), 6, 1, 3, 1),
        b.place(queryValue('Function errors', [metric(`sum:gcp.cloudfunctions.function.execution_count{${f},!status:ok}`, 'sum')], { formats: BAD }), 9, 1, 3, 1),
        b.place(timeseries('Requests by service', [metric(`sum:gcp.run.request_count{${f}} by {service_name}`, 'sum')], { display: 'bars', palette: 'purple' }), 0, 2, 6, 3),
        b.place(timeseries('Executions by function', [metric(`sum:gcp.cloudfunctions.function.execution_count{${f}} by {function_name}`, 'sum')], { display: 'bars', palette: 'purple' }), 6, 2, 6, 3),
      ],
      4,
      6,
    ),
    b.group(
      'Load Balancers',
      'yellow',
      [
        b.place(section('External HTTP(S) load balancers: requests and bytes.', 'yellow'), 0, 0, 12, 1),
        b.place(queryValue('Requests', [metric(`sum:gcp.loadbalancing.https.request_count{${f}}`, 'sum')]), 0, 1, 4, 1),
        b.place(queryValue('Bytes received', [metric(`sum:gcp.loadbalancing.https.request_bytes_count{${f}}`, 'sum')]), 0, 2, 4, 1),
        b.place(queryValue('Bytes sent', [metric(`sum:gcp.loadbalancing.https.response_bytes_count{${f}}`, 'sum')]), 0, 3, 4, 1),
        b.place(timeseries('Requests by URL map', [metric(`sum:gcp.loadbalancing.https.request_count{${f}} by {url_map_name}`, 'sum')], { display: 'bars', palette: 'orange' }), 4, 1, 8, 3),
      ],
      5,
      5,
    ),
  ];
  return {
    builder: b,
    doc: {
      title: 'Google Cloud Overview',
      description: 'The Google Cloud services ObserveCraft collects from Cloud Monitoring: Compute Engine, Cloud SQL, Kubernetes Engine, Cloud Run, Cloud Functions and load balancing.',
      layout_type: 'ordered',
      template_variables: [
        { name: 'project', prefix: 'project_id', default: '*' },
        { name: 'zone', prefix: 'zone', default: '*' },
        { name: 'region', prefix: 'region', default: '*' },
      ],
      widgets,
    },
  };
}

/** Units of the metrics the dashboards chart (Datadog unit names): the crawlers send none, and the
 * server reads them from server/seeds/integrations/cloud/units.csv. Counts have none. */
export const CLOUD_UNITS = {
  'aws.ec2.cpuutilization': 'percent',
  'aws.ec2.cpuutilization.maximum': 'percent',
  'aws.ec2.network_in.sum': 'byte',
  'aws.ec2.network_out.sum': 'byte',
  'aws.rds.cpuutilization': 'percent',
  'aws.rds.free_storage_space': 'byte',
  'aws.rds.read_latency': 'second',
  'aws.rds.write_latency': 'second',
  'aws.elb.latency': 'second',
  'aws.applicationelb.target_response_time': 'second',
  'aws.lambda.duration': 'millisecond',
  'aws.s3.bucket_size_bytes': 'byte',
  'aws.ebs.burst_balance': 'percent',
  'aws.ebs.volume_idle_time.sum': 'second',
  'aws.ebs.volume_read_bytes.sum': 'byte',
  'aws.ebs.volume_write_bytes.sum': 'byte',
  'azure.vm.percentage_cpu': 'percent',
  'azure.vm.percentage_cpu.max': 'percent',
  'azure.vm.network_in_total': 'byte',
  'azure.vm.network_out_total': 'byte',
  'azure.vm.available_memory_bytes': 'byte',
  'azure.vm.disk_read_bytes': 'byte',
  'azure.vm.disk_write_bytes': 'byte',
  'azure.sql_servers_databases.cpu_percent': 'percent',
  'azure.sql_servers_databases.storage_percent': 'percent',
  'azure.sql_servers_databases.dtu_consumption_percent': 'percent',
  'azure.app_services.http_response_time': 'second',
  'azure.network_loadbalancers.vip_availability': 'percent',
  'azure.network_loadbalancers.dip_availability': 'percent',
  'azure.network_loadbalancers.byte_count': 'byte',
  'azure.storage.availability': 'percent',
  'azure.storage.used_capacity': 'byte',
  'azure.storage.success_server_latency': 'millisecond',
  'gcp.gce.instance.cpu.utilization': 'fraction',
  'gcp.gce.instance.network.received_bytes_count': 'byte',
  'gcp.gce.instance.network.sent_bytes_count': 'byte',
  'gcp.gce.instance.disk.read_bytes_count': 'byte',
  'gcp.cloudsql.database.cpu.utilization': 'fraction',
  'gcp.cloudsql.database.memory.utilization': 'fraction',
  'gcp.cloudsql.database.disk.bytes_used': 'byte',
  'gcp.cloudsql.database.disk.utilization': 'fraction',
  'gcp.gke.node.cpu.allocatable_utilization': 'fraction',
  'gcp.gke.node.memory.allocatable_utilization': 'fraction',
  'gcp.gke.container.memory.used_bytes': 'byte',
  'gcp.loadbalancing.https.request_bytes_count': 'byte',
  'gcp.loadbalancing.https.response_bytes_count': 'byte',
};

export const unitsCsv = () =>
  `# Units of the cloud overview dashboards' metrics (scripts/cloud-dashboards.mjs; the crawlers send none).\n${Object.entries(CLOUD_UNITS)
    .map(([name, unit]) => `${name},${unit}`)
    .join('\n')}\n`;

// ---------------------------------------------------------------- conversion

/** Replaces the placeholders of the widgets the converter doesn't know, and names ids after the cloud. */
function finish(widgets, overrides, cloud) {
  return widgets.map((w) => {
    const n = Number(w.id.replace(/^upstream-/, ''));
    const definition = overrides.has(n) ? overrides.get(n) : w.definition.type === 'group' ? { ...w.definition, widgets: finish(w.definition.widgets, overrides, cloud) } : w.definition;
    return { ...w, id: w.id.replace(/^upstream-/, `${cloud}-`), definition };
  });
}

/** Rows (40px) a widget takes: section notes 2, tips and the intro 3, query values 3, tiles 4, charts two per source unit. */
function rows(definition, source) {
  // Section notes are one line; tips beside query values and the intro take their height.
  if (definition.type === 'note') return source.height === 1 && definition.align !== 'left' ? 2 : 3;
  if (definition.type === 'integration' && definition.original.type === 'query_value') return 3 * source.height;
  return 2 * source.height;
}

/** Places each widget, in reading order, as high as it fits below the ones before it (like an ordered layout). */
function compact(widgets) {
  const placed = [];
  for (const w of [...widgets].sort((a, b) => a.layout.y - b.layout.y || a.layout.x - b.layout.x)) {
    w.layout.y = 0;
    for (;;) {
      const a = w.layout;
      const hits = placed.filter(({ layout: b }) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y);
      if (!hits.length) break;
      a.y = Math.max(...hits.map(({ layout: b }) => b.y + b.h));
    }
    placed.push(w);
  }
  return widgets;
}

/** Heights in our rows instead of the converter's (which gives every widget at least four rows). */
function relayout(widgets, layouts, cloud) {
  return compact(
    widgets.map((w) => {
      const source = layouts.get(Number(w.id.slice(cloud.length + 1)));
      if (!source) return w; // a group's banner keeps its place
      let definition = w.definition;
      const layout = { ...w.layout, y: source.y * 2 };
      if (definition.type === 'group') {
        const children = relayout(definition.widgets, layouts, cloud);
        definition = { ...definition, widgets: children };
        // As the canvas sizes groups (tree.ts groupHeight): three rows of header, one without a title.
        layout.h = (definition.showTitle === false ? 1 : 3) + Math.max(2, ...children.map((c) => c.layout.y + c.layout.h));
      } else {
        layout.h = rows(definition, source);
      }
      return { ...w, definition, layout };
    }),
  );
}

export function buildDashboards() {
  return [aws(), azure(), gcp()].map(({ builder, doc }, i) => {
    const cloud = CLOUDS[i];
    const converted = convertDashboard(doc, cloud, `${cloud}_overview.json`, 'observecraft');
    return {
      ...converted,
      id: `${cloud}-overview`,
      description: doc.description,
      widgets: relayout(finish(converted.widgets, builder.overrides, cloud), builder.layouts, cloud),
      source: { repository: 'observecraft', path: 'scripts/cloud-dashboards.mjs', license: 'MIT' },
    };
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  for (const d of buildDashboards()) {
    const dir = new URL(`../server/seeds/integrations/${d.logo}/`, import.meta.url);
    mkdirSync(dir, { recursive: true });
    writeFileSync(new URL('dashboards.json', dir), JSON.stringify([d], null, 2) + '\n');
  }
  mkdirSync(new URL('../server/seeds/integrations/cloud/', import.meta.url), { recursive: true });
  writeFileSync(new URL('../server/seeds/integrations/cloud/units.csv', import.meta.url), unitsCsv());
  console.log('wrote', CLOUDS.map((c) => `server/seeds/integrations/${c}/dashboards.json`).join(', '));
}
