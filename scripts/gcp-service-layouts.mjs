// Dashboards designed per Google Cloud service, as Datadog has one for each service of its Google Cloud integration:
// each follows its service's model (what a revision, a subscription or a URL map is judged by) instead of one template.
// The structure (an overview with the service's logo, key numbers and monitors, then a group per question, its logs
// last) follows the kind of dashboards Datadog ships; titles, notes and layouts are ours, and only names our GCP crawler
// publishes are charted (gcp/naming.rs: gauges and deltas under the bare name, cumulative counters' change as `.delta`,
// distributions as `.avg` and, for the latencies Datadog has them for, `.p95` / `.p99`). Series carry the project, the
// resource's and the metric's labels, the resource's user labels and a zone's region (gcp/crawler.rs series_tags);
// `labels` lists the Cloud Monitoring labels each layout splits or filters by, and scripts/cloud-service-dashboards.mjs
// refuses any other. The services left out get its generated layout.
import { chg, group, heat, qv, tbl, top, tree, ts, txt } from './aws-service-layouts.mjs';
import { metric } from './cloud-dashboards.mjs';

// ---------------------------------------------------------------- widgets

/** A themed group, in Google Cloud's blue. */
const g = (title, widgets, o = {}) => group(title, widgets, { color: 'blue', ...o });
/** Each group's share of a total, as rings. */
const pie = (title, q, { agg = 'sum', w = 4, h = 2 } = {}) => ({
  definition: {
    type: 'sunburst',
    title,
    title_size: '16',
    title_align: 'left',
    requests: [{ response_format: 'scalar', queries: [metric(q, agg)], formulas: [{ formula: 'query1' }] }],
    legend: { type: 'automatic' },
  },
  width: w,
  height: h,
});

/** The queries of a service's metrics, by their name after `gcp.<service>.` (`instance.cpu.utilization`). */
const on = (q) => {
  const named =
    (space) =>
    (name, ...rest) =>
      q[space](`gcp.${q.short}.${name}`, ...rest);
  return { sum: named('sum'), avg: named('avg'), max: named('max'), min: named('min') };
};

/** A distribution's tail and mean: its `.p99` and `.p95` (the worst series) against its `.avg`. */
const tail = (title, { max, avg }, base, o = {}) => ts(title, [max(`${base}.p99`), max(`${base}.p95`), avg(`${base}.avg`)], { alias: ['p99', 'p95', 'avg'], ...o });
/** A part's share of a whole, in percent, per group of both. */
const share = (title, part, whole, o = {}) => ts(title, [part, whole], { formulas: [['100 * query1 / query2', '%']], palette: 'warm', ...o });
/** In against out, mirrored around zero. */
const mirror = (title, inbound, outbound, o = {}) =>
  ts(title, [inbound, outbound], {
    formulas: [
      ['query1', 'in'],
      ['0 - query2', 'out'],
    ],
    display: 'bars',
    palette: 'cool',
    ...o,
  });
/** How many resources report a gauge (one series each). */
const reporting = (title, q, o = {}) => qv(title, q, { formula: 'count_not_null(query1)', good: true, ...o });

// ---------------------------------------------------------------- the services

/**
 * service prefix (gcp_integration.json) → { vars: the template variables after the project, labels: the Cloud
 * Monitoring resource and metric labels its charts split or filter by, monitor: the `integration:` tag of its monitors,
 * note: what it shows (our words), logs: [[title, query]], build({ sum, avg, max, min, short, f }) → { kpis, groups } }.
 */
export const GCP_LAYOUTS = {
  'compute.googleapis.com/': {
    vars: ['region', 'zone', 'instance_name'],
    labels: ['instance_id', 'instance_name', 'device_name', 'storage_type', 'instance_group_name'],
    monitor: 'google_compute_engine',
    note: 'VM instances seen from the hypervisor: CPU and the time vCPUs wait to run, persistent disk throughput and latency, VPC traffic and firewall drops, and managed instance groups. Memory and file systems inside the guest come from the Agent or the Ops Agent.',
    logs: [['Instance logs', 'source:gcp.gce.instance']],
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          reporting('Instances reporting', avg('instance.cpu.utilization', 'instance_id')),
          qv('Avg CPU', avg('instance.cpu.utilization'), { precision: 2 }),
          qv('Busiest instance CPU', max('instance.cpu.utilization'), { precision: 2 }),
          qv('Received', sum('instance.network.received_bytes_count')),
          qv('Sent', sum('instance.network.sent_bytes_count')),
          qv('Firewall drops', sum('firewall.dropped_packets_count'), { bad: true }),
        ],
        groups: [
          g('CPU and scheduling', [
            top('Busiest instances', avg('instance.cpu.utilization', 'instance_name'), { w: 4, h: 4, palette: 'orange' }),
            ts('CPU utilization by instance', avg('instance.cpu.utilization', 'instance_name'), { w: 8 }),
            heat('CPU spread across instances', avg('instance.cpu.utilization', 'instance_name'), { w: 4 }),
            ts('vCPU wait for a physical CPU', sum('instance.cpu.scheduler_wait_time', 'instance_name'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
          g('Persistent disks', [
            ts('Disk throughput', [sum('instance.disk.read_bytes_count'), sum('instance.disk.write_bytes_count')], { alias: ['read', 'written'], w: 4 }),
            ts('Disk operations', [sum('instance.disk.read_ops_count'), sum('instance.disk.write_ops_count')], { alias: ['reads', 'writes'], w: 4 }),
            ts('Average I/O latency by instance', max('instance.disk.average_io_latency', 'instance_name'), { w: 4 }),
            top('Deepest I/O queues', max('instance.disk.average_io_queue_depth', 'instance_name'), { agg: 'avg' }),
            top('Most written by device', sum('instance.disk.write_bytes_count', 'device_name')),
          ]),
          g('VPC traffic and firewall', [
            mirror('Traffic in and out', sum('instance.network.received_bytes_count'), sum('instance.network.sent_bytes_count')),
            ts('Packets in and out', [sum('instance.network.received_packets_count'), sum('instance.network.sent_packets_count')], { alias: ['in', 'out'] }),
            top('Most traffic sent', sum('instance.network.sent_bytes_count', 'instance_name'), { w: 4 }),
            ts('Packets dropped by firewall rules', sum('firewall.dropped_packets_count', 'instance_name'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Drops for lack of receive buffers', sum('network.dropped_packets.no_vm_receive_buffers_count', 'instance_name'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
          g('Memory, uptime and instance groups', [
            ts('Guest memory used (balloon driver)', [sum('instance.memory.balloon.ram_used'), sum('instance.memory.balloon.ram_size')], { alias: ['used', 'size'], w: 4 }),
            top('Most recently started', min('instance.uptime_total', 'instance_name'), { order: 'asc', agg: 'last', w: 4 }),
            ts('Managed instance group size', avg('instance_group.size', 'instance_group_name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'kubernetes.io/': {
    vars: ['location', 'cluster_name', 'namespace_name', 'node_name'],
    labels: ['cluster_name', 'namespace_name', 'pod_name', 'container_name', 'node_name', 'memory_type', 'volume_name'],
    monitor: 'google_kubernetes_engine',
    note: 'GKE clusters from Cloud Monitoring’s system metrics: each node’s allocatable CPU and memory against what pods use, containers against their requests and limits, restarts, pod volumes and network, and kube-dns. Workloads in detail come from the Agent (Kubernetes Explorer).',
    logs: [['Container logs', 'source:gcp.k8s.container']],
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          reporting('Nodes reporting', avg('node.cpu.allocatable_utilization', 'node_name')),
          qv('Busiest node CPU', max('node.cpu.allocatable_utilization'), { precision: 2 }),
          qv('Busiest node memory', max('node.memory.allocatable_utilization'), { precision: 2 }),
          qv('Container restarts', sum('container.restart_count.delta'), { bad: true }),
          qv('Fullest pod volume', max('pod.volume.utilization'), { precision: 2 }),
        ],
        groups: [
          g('Nodes: allocatable against used', [
            top('Busiest nodes', max('node.cpu.allocatable_utilization', 'node_name'), { agg: 'avg', w: 4, h: 4, palette: 'orange' }),
            ts('CPU used of allocatable, by node', max('node.cpu.allocatable_utilization', 'node_name'), { w: 8 }),
            ts('Memory used of allocatable, by node', max('node.memory.allocatable_utilization', 'node_name'), { w: 4 }),
            ts('Allocatable and total cores', [sum('node.cpu.allocatable_cores'), sum('node.cpu.total_cores')], { alias: ['allocatable', 'total'], w: 4 }),
            ts('Node memory allocatable and in use', [sum('node.memory.allocatable_bytes'), sum('node.memory.used_bytes', null, 'memory_type:non-evictable')], {
              alias: ['allocatable', 'used'],
              w: 4,
            }),
            top('Fullest node ephemeral storage', [max('node.ephemeral_storage.used_bytes', 'node_name'), max('node.ephemeral_storage.allocatable_bytes', 'node_name')], {
              formula: '100 * query1 / query2',
              agg: 'avg',
              w: 4,
            }),
          ]),
          g('Containers: requests, limits and restarts', [
            top('Most restarted pods', sum('container.restart_count.delta', 'pod_name'), { palette: 'warm', w: 4 }),
            ts('Restarts by namespace', sum('container.restart_count.delta', 'namespace_name'), { display: 'bars', palette: 'warm', w: 8 }),
            ts('CPU used of requested, by container', max('container.cpu.request_utilization', 'container_name'), { w: 4 }),
            ts('Memory used of the limit, by container', max('container.memory.limit_utilization', 'container_name'), { w: 4 }),
            ts('CPU time by namespace', sum('container.cpu.core_usage_time.delta', 'namespace_name'), { display: 'bars', w: 4 }),
            ts('Memory in use by namespace', sum('container.memory.used_bytes', 'namespace_name', 'memory_type:non-evictable')),
            ts('Page faults by namespace', sum('container.memory.page_fault_count.delta', 'namespace_name')),
          ]),
          g('Pods: volumes, network and startup', [
            top('Fullest pod volumes', max('pod.volume.utilization', 'pod_name'), { agg: 'avg', w: 4 }),
            ts('Volume space used by volume', max('pod.volume.used_bytes', 'volume_name'), { w: 4 }),
            ts('Time to the first ready state', avg('pod.latencies.pod_first_ready', 'namespace_name'), { w: 4 }),
            mirror('Pod traffic in and out', sum('pod.network.received_bytes_count.delta'), sum('pod.network.sent_bytes_count.delta')),
            mirror('Node traffic in and out', sum('node.network.received_bytes_count.delta'), sum('node.network.sent_bytes_count.delta')),
          ]),
          g('Cluster DNS (kube-dns)', [
            ts('DNS requests by cluster', sum('networking.dns.kubedns.dns_request_count', 'cluster_name'), { display: 'bars', w: 4 }),
            ts('DNS request latency', avg('networking.dns.kubedns.dns_request_latencies.avg', 'cluster_name'), { w: 4 }),
            ts('Requests rejected at max concurrency', sum('networking.dns.kubedns.max_concurrent_rejected_request_count', 'cluster_name'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },

  'run.googleapis.com/': {
    vars: ['location', 'service_name', 'revision_name'],
    labels: ['service_name', 'revision_name', 'configuration_name', 'response_code', 'response_code_class', 'route', 'state'],
    monitor: 'google_cloud_run',
    note: 'Cloud Run services by revision: requests by response class and their latency percentiles, container instances and how many requests each takes, CPU and memory utilization across instances, billable time and Cloud SQL connections.',
    logs: [['Cloud Run logs', 'source:gcp.cloud.run.revision']],
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Requests', sum('request_count')),
          qv('5xx responses', sum('request_count', null, 'response_code_class:5xx'), { bad: true }),
          qv('p95 latency', max('request_latencies.p95'), { precision: 1 }),
          qv('Container instances', sum('container.instance_count'), { agg: 'last' }),
          qv('Billable instance time', sum('container.billable_instance_time')),
          qv('CPU p99', max('container.cpu.utilizations.p99'), { precision: 2 }),
        ],
        groups: [
          g('Requests by response class', [
            ts('Requests by response class', sum('request_count', 'response_code_class'), { display: 'bars', w: 8 }),
            top('Busiest services', sum('request_count', 'service_name'), { w: 4 }),
            share('5xx share by service', sum('request_count', 'service_name', 'response_code_class:5xx'), sum('request_count', 'service_name')),
            top('Most 4xx and 5xx by revision', sum('request_count', 'revision_name', '!response_code_class:2xx', '!response_code_class:3xx'), { palette: 'warm' }),
          ]),
          g('Request latency', [
            tail('Request latency', on(q), 'request_latencies', { w: 4 }),
            ts('p99 by service', max('request_latencies.p99', 'service_name'), { w: 4 }),
            top('Slowest services (p95)', max('request_latencies.p95', 'service_name'), { agg: 'avg', w: 4 }),
            ts(
              'Where a request’s time goes',
              [avg('request_latency.pending.avg'), avg('request_latency.routing.avg'), avg('request_latency.user_execution.avg'), avg('request_latency.response_egress.avg')],
              { alias: ['pending', 'routing', 'your code', 'response egress'], w: 12 },
            ),
          ]),
          g('Instances and concurrency', [
            ts('Instances by state', sum('container.instance_count', 'state'), { display: 'bars', w: 4 }),
            ts('Instances by service', sum('container.instance_count', 'service_name'), { w: 4 }),
            ts('Recommended instances', sum('scaling.recommended_instances', 'service_name'), { w: 4 }),
            tail('Concurrent requests per instance', on(q), 'container.max_request_concurrencies', { w: 6 }),
            ts('Container startup latency', avg('container.startup_latencies.avg', 'service_name'), { w: 6 }),
          ]),
          g('Container CPU and memory', [
            tail('CPU utilization across instances', on(q), 'container.cpu.utilizations', { w: 4 }),
            tail('Memory utilization across instances', on(q), 'container.memory.utilizations', { w: 4 }),
            ts('CPU p95 by revision', max('container.cpu.utilizations.p95', 'revision_name'), { w: 4 }),
            top('Most memory used (p99)', max('container.memory.utilizations.p99', 'service_name'), { agg: 'avg' }),
            mirror('Network in and out', sum('container.network.received_bytes_count'), sum('container.network.sent_bytes_count')),
          ]),
          g('Billing and Cloud SQL connections', [
            ts('Billable instance time by service', sum('container.billable_instance_time', 'service_name'), { display: 'bars', w: 6 }),
            ts('CPU allocated (seconds)', sum('container.cpu.allocation_time', 'service_name'), { display: 'bars', w: 6 }),
            ts('Cloud SQL connections opened and refused', [sum('infrastructure.cloudsql.connection_request_count'), sum('infrastructure.cloudsql.connection_refused_count')], {
              alias: ['requested', 'refused'],
              w: 6,
            }),
            ts('Cloud SQL connection latency', avg('infrastructure.cloudsql.connection_latencies.avg', 'service_name'), { w: 6 }),
          ]),
        ],
      };
    },
  },

  'cloudfunctions.googleapis.com/': {
    vars: ['region', 'function_name'],
    labels: ['function_name', 'status', 'trigger_type', 'memory', 'state'],
    monitor: 'google_cloud_functions',
    note: 'Cloud Functions (1st gen) by function: executions and how they ended, execution time percentiles, active instances, memory used per execution and outbound traffic. 2nd gen functions run on Cloud Run and show there.',
    logs: [['Function logs', 'source:gcp.cloud.function']],
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Executions', sum('function.execution_count')),
          qv('Failed executions', sum('function.execution_count', null, '!status:ok'), { bad: true }),
          qv('p95 execution time', max('function.execution_times.p95'), { precision: 0 }),
          qv('Active instances', sum('function.active_instances'), { agg: 'last' }),
          qv('Egress', sum('function.network_egress')),
        ],
        groups: [
          g('Executions and how they ended', [
            ts('Executions by status', sum('function.execution_count', 'status'), { display: 'bars', w: 8 }),
            pie('By trigger', sum('function.execution_count', 'trigger_type')),
            share('Failure share by function', sum('function.execution_count', 'function_name', '!status:ok'), sum('function.execution_count', 'function_name')),
            top('Most invoked functions', sum('function.execution_count', 'function_name')),
          ]),
          g('Execution time', [
            tail('Execution time', on(q), 'function.execution_times', { w: 4 }),
            top('Slowest functions (p95)', max('function.execution_times.p95', 'function_name'), { agg: 'avg', w: 4 }),
            ts('p99 by function', max('function.execution_times.p99', 'function_name'), { w: 4 }),
          ]),
          g('Instances and memory', [
            ts('Active instances by function', sum('function.active_instances', 'function_name'), { w: 4 }),
            ts('Instances by state', sum('function.instance_count', 'state'), { display: 'bars', w: 4 }),
            tail('Memory used per execution', on(q), 'function.user_memory_bytes', { w: 4 }),
            top('Most memory used (p99)', max('function.user_memory_bytes.p99', 'function_name'), { agg: 'avg' }),
            ts('Egress by function', sum('function.network_egress', 'function_name'), { display: 'bars' }),
          ]),
        ],
      };
    },
  },

  'appengine.googleapis.com/': {
    vars: ['module_id', 'version_id'],
    labels: ['module_id', 'version_id', 'response_code', 'loading'],
    monitor: 'google_app_engine',
    note: 'App Engine services and versions: responses by status code and their latency percentiles, instances and the estimate you are billed for, the flexible environment’s CPU and connections, and the shared memcache.',
    logs: [['App Engine logs', 'source:gcp.gae.app']],
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Responses', sum('http.server.response_count')),
          qv('p95 latency', max('http.server.response_latencies.p95'), { precision: 0 }),
          qv('Instances', sum('system.instance_count'), { agg: 'last' }),
          qv('Billed instances (estimate)', sum('system.billed_instance_estimate_count'), { agg: 'last' }),
          qv('Quota denials', sum('http.server.quota_denial_count'), { bad: true }),
          qv('DoS intercepts', sum('http.server.dos_intercept_count'), { bad: true }),
        ],
        groups: [
          g('HTTP responses', [
            ts('Responses by status code', sum('http.server.response_count', 'response_code'), { display: 'bars', w: 8 }),
            top('Busiest services', sum('http.server.response_count', 'module_id'), { w: 4 }),
            ts('Requests served by a new instance', sum('http.server.response_count', 'loading'), { display: 'bars' }),
            ts('Denied for quota or intercepted', [sum('http.server.quota_denial_count'), sum('http.server.dos_intercept_count')], { alias: ['quota', 'DoS'], display: 'bars', palette: 'warm' }),
          ]),
          g('Response latency', [
            tail('Response latency', on(q), 'http.server.response_latencies', { w: 6 }),
            ts('p95 by service', max('http.server.response_latencies.p95', 'module_id'), { w: 6 }),
            ts('Pending requests', sum('system.pending_queue.pending_requests', 'module_id'), { w: 12 }),
          ]),
          g('Instances and billing', [
            ts('Instances by service', sum('system.instance_count', 'module_id'), { w: 4 }),
            ts('Billed instance estimate', sum('system.billed_instance_estimate_count', 'module_id'), { w: 4 }),
            ts('CPU utilization by version', avg('system.cpu.utilization', 'version_id'), { w: 4 }),
            ts('Memory by service', sum('system.memory.usage', 'module_id')),
            mirror('Traffic in and out', sum('system.network.received_bytes_count'), sum('system.network.sent_bytes_count')),
          ]),
          g('Flexible environment', [
            ts('CPU utilization (flex)', avg('flex.cpu.utilization', 'version_id'), { w: 4 }),
            ts('Open connections (flex)', avg('flex.connections.current', 'version_id'), { w: 4 }),
            ts('Autoscaler utilization against capacity', [avg('flex.autoscaler.current_utilization'), avg('flex.autoscaler.capacity')], { alias: ['utilization', 'capacity'], w: 4 }),
          ]),
          g('Memcache', [
            qv('Hit ratio', avg('memcache.hit_ratio'), { precision: 2 }),
            ts('Hits and misses', [sum('memcache.hit_count'), sum('memcache.miss_count')], { alias: ['hits', 'misses'], w: 5 }),
            ts('Cache used and its limit', [max('memcache.used_cache_size'), max('memcache.cache_size_limit')], { alias: ['used', 'limit'], w: 5 }),
          ]),
        ],
      };
    },
  },

  'cloudsql.googleapis.com/': {
    vars: ['region', 'database_id'],
    labels: ['database_id', 'database', 'data_type', 'state'],
    monitor: 'google_cloudsql',
    note: 'Cloud SQL instances of every engine in one place: availability, failover and replica lag, CPU, memory and disk, connections, then what each engine reports: MySQL’s queries and InnoDB, PostgreSQL’s backends, transactions and Query Insights, SQL Server’s batches and buffer pool. Groups of an engine you don’t run stay empty.',
    logs: [['Cloud SQL logs', 'source:gcp.cloudsql.database']],
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Instances up', sum('database.up'), { agg: 'last', good: true }),
          qv('Avg CPU', avg('database.cpu.utilization'), { precision: 2 }),
          qv('Busiest memory', max('database.memory.utilization'), { precision: 2 }),
          qv('Fullest disk', max('database.disk.utilization'), { precision: 2 }),
          qv('Connections', sum('database.network.connections'), { agg: 'last' }),
          qv('Max replica lag', max('database.replication.replica_lag'), { precision: 1 }),
        ],
        groups: [
          g('Availability, failover and replicas', [
            ts('Up by instance', min('database.up', 'database_id'), { w: 4 }),
            ts('Replica lag', max('database.replication.replica_lag', 'database_id'), { w: 4 }),
            ts('Replication network lag', max('database.replication.network_lag', 'database_id'), { w: 4 }),
            ts('Ready for failover (HA)', min('database.available_for_failover', 'database_id'), { w: 6 }),
            ts('Automatic failovers requested', sum('database.auto_failover_request_count', 'database_id'), { display: 'bars', palette: 'warm', w: 6 }),
          ]),
          g('CPU and memory', [
            top('Busiest instances', avg('database.cpu.utilization', 'database_id'), { w: 4, h: 4, palette: 'orange' }),
            ts('CPU utilization by instance', avg('database.cpu.utilization', 'database_id'), { w: 8 }),
            ts('Memory utilization by instance', max('database.memory.utilization', 'database_id'), { w: 4 }),
            ts('Swap used', max('database.swap.bytes_used', 'database_id'), { w: 4 }),
          ]),
          g('Disk and I/O', [
            top('Fullest disks', max('database.disk.utilization', 'database_id'), { agg: 'avg', w: 4 }),
            ts('Disk used by data type', max('database.disk.bytes_used_by_data_type', 'data_type'), { w: 4 }),
            ts('Disk operations', [sum('database.disk.read_ops_count'), sum('database.disk.write_ops_count')], { alias: ['reads', 'writes'], w: 4 }),
            ts('Disk used against its size', [sum('database.disk.bytes_used'), sum('database.disk.quota')], { alias: ['used', 'size'] }),
            ts('Data cache used (Enterprise Plus)', max('database.data_cache.bytes_used', 'database_id')),
          ]),
          g('Connections and traffic', [
            ts('Connections by instance', sum('database.network.connections', 'database_id'), { w: 4 }),
            ts('Connection attempts', sum('database.network.connection_attempt_count', 'database_id'), { display: 'bars', w: 4 }),
            mirror('Traffic in and out', sum('database.network.received_bytes_count'), sum('database.network.sent_bytes_count'), { w: 4 }),
          ]),
          g('MySQL', [
            ts('Queries and statements from clients', [sum('database.mysql.queries'), sum('database.mysql.questions')], { alias: ['queries', 'questions'], w: 4 }),
            ts('Slow queries', sum('database.mysql.slow_queries_count', 'database_id'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Seconds behind the primary', max('database.mysql.replication.seconds_behind_master', 'database_id'), { w: 4 }),
            ts('InnoDB row lock waits and deadlocks', [sum('database.mysql.innodb.row_lock_waits_count'), sum('database.mysql.innodb.deadlocks_count')], {
              alias: ['lock waits', 'deadlocks'],
              palette: 'warm',
              w: 4,
            }),
            ts('Threads', sum('database.mysql.threads', 'database_id'), { w: 4 }),
            ts('Aborted connections', sum('database.mysql.aborted_connects_count', 'database_id'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
          g('PostgreSQL', [
            ts('Backends by database', sum('database.postgresql.num_backends', 'database'), { w: 4 }),
            ts('Transactions by database', sum('database.postgresql.transaction_count', 'database'), { display: 'bars', w: 4 }),
            ts('Deadlocks', sum('database.postgresql.deadlock_count', 'database'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Buffer cache hit ratio', min('database.postgresql.data_cache.hit_ratio', 'database_id'), { w: 4 }),
            top('Closest to transaction ID wraparound', max('database.postgresql.transaction_id_utilization', 'database_id'), { agg: 'avg', w: 4 }),
            ts('Replica byte lag', max('database.postgresql.replication.replica_byte_lag', 'database_id'), { w: 4 }),
            tail('Query Insights: query latency', on(q), 'database.postgresql.insights.aggregate.latencies', { w: 6 }),
            ts('Query Insights: lock and I/O time', [sum('database.postgresql.insights.aggregate.lock_time.delta'), sum('database.postgresql.insights.aggregate.io_time.delta')], {
              alias: ['lock', 'I/O'],
              display: 'bars',
              w: 6,
            }),
          ]),
          g('SQL Server', [
            ts('Batch requests', sum('database.sqlserver.transactions.batch_request_count', 'database_id'), { display: 'bars', w: 4 }),
            ts('Deadlocks and lock waits', [sum('database.sqlserver.transactions.deadlock_count'), sum('database.sqlserver.transactions.lock_wait_count')], {
              alias: ['deadlocks', 'lock waits'],
              palette: 'warm',
              w: 4,
            }),
            ts('Blocked processes', max('database.sqlserver.connections.processes_blocked', 'database_id'), { w: 4 }),
            ts('Page life expectancy', min('database.sqlserver.memory.page_life_expectancy', 'database_id'), { w: 6 }),
            ts('Buffer cache hit ratio', min('database.sqlserver.memory.buffer_cache_hit_ratio', 'database_id'), { w: 6 }),
          ]),
        ],
      };
    },
  },

  'spanner.googleapis.com/': {
    vars: ['instance_config', 'instance_id'],
    labels: ['instance_id', 'instance_config', 'database', 'method', 'status', 'priority', 'storage_class'],
    monitor: 'google_cloud_spanner',
    note: 'Spanner instances: CPU by priority against the recommended maximum, API requests and their latency by method, commits, retries and lock waits, queries, storage per database and the compute capacity (nodes, processing units) autoscaling gives them.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Avg CPU', avg('instance.cpu.utilization'), { precision: 2 }),
          qv('Peak high-priority CPU', max('instance.cpu.utilization_by_priority', null, 'priority:high'), { precision: 2 }),
          qv('Fullest storage', max('instance.storage.utilization'), { precision: 2 }),
          qv('API requests', sum('api.api_request_count')),
          qv('Failed queries', sum('query_stat.total.failed_execution_count'), { bad: true }),
          qv('Processing units', sum('instance.processing_units'), { agg: 'last' }),
        ],
        groups: [
          g('CPU by priority', [
            ts('CPU by priority', max('instance.cpu.utilization_by_priority', 'priority'), { w: 4 }),
            ts(
              'High-priority CPU against its autoscaling target',
              [max('instance.cpu.utilization_by_priority', null, 'priority:high'), avg('instance.autoscaling.high_priority_cpu_utilization_target')],
              {
                alias: ['high priority', 'target'],
                w: 4,
              },
            ),
            top('Busiest instances (24 h smoothed)', max('instance.cpu.smoothed_utilization', 'instance_id'), { agg: 'avg', w: 4 }),
          ]),
          g('API requests and latency', [
            ts('Requests by status', sum('api.api_request_count', 'status'), { display: 'bars', w: 4 }),
            top('Busiest methods', sum('api.api_request_count', 'method'), { w: 4 }),
            ts('Latency by method', avg('api.request_latencies.avg', 'method'), { w: 4 }),
            mirror('Bytes received and sent', sum('api.received_bytes_count'), sum('api.sent_bytes_count'), { w: 12 }),
          ]),
          g('Transactions and locks', [
            ts('Commit attempts and retries', [sum('transaction_stat.total.commit_attempt_count'), sum('transaction_stat.total.commit_retry_count')], { alias: ['attempts', 'retries'], w: 4 }),
            ts('Transaction latency', avg('transaction_stat.total.transaction_latencies.avg', 'database'), { w: 4 }),
            ts('Lock wait time', sum('lock_stat.total.lock_wait_time', 'database'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
          g('Queries', [
            ts('Queries run and failed', [sum('query_stat.total.execution_count'), sum('query_stat.total.failed_execution_count')], { alias: ['run', 'failed'], w: 4 }),
            ts('Query latency', avg('query_stat.total.query_latencies.avg', 'database'), { w: 4 }),
            ts('Rows scanned and returned', [sum('query_stat.total.scanned_rows_count'), sum('query_stat.total.returned_rows_count')], { alias: ['scanned', 'returned'], w: 4 }),
          ]),
          g('Storage and compute capacity', [
            ts('Storage by database', max('instance.storage.used_bytes', 'database'), { w: 4 }),
            ts('Storage used of the limit', max('instance.storage.utilization', 'instance_id'), { w: 4 }),
            ts('Processing units and autoscaling bounds', [sum('instance.processing_units'), sum('instance.autoscaling.min_processing_units'), sum('instance.autoscaling.max_processing_units')], {
              alias: ['current', 'min', 'max'],
              w: 4,
            }),
            ts('Nodes by instance', sum('instance.node_count', 'instance_id')),
            ts('Sessions by database', sum('instance.session_count', 'database')),
          ]),
        ],
      };
    },
  },

  'bigtable.googleapis.com/': {
    vars: ['instance', 'cluster', 'table'],
    labels: ['instance', 'cluster', 'table', 'app_profile', 'method', 'error_code'],
    monitor: 'google_cloud_bigtable',
    note: 'Bigtable instances by cluster and table: CPU load and the hottest node, server requests, errors and latency, table and disk storage, replication delay and what client libraries see (operation latency, retries).',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Avg cluster CPU', avg('cluster.cpu_load'), { precision: 2 }),
          qv('Hottest node CPU', max('cluster.cpu_load_hottest_node'), { precision: 2 }),
          qv('Requests', sum('server.request_count')),
          qv('Server errors', sum('server.error_count'), { bad: true }),
          qv('Fullest cluster storage', max('cluster.storage_utilization'), { precision: 2 }),
        ],
        groups: [
          g('Cluster load and nodes', [
            ts('CPU load by cluster', avg('cluster.cpu_load', 'cluster'), { w: 4 }),
            ts('Hottest node by cluster', max('cluster.cpu_load_hottest_node', 'cluster'), { w: 4 }),
            ts('Nodes and recommended for CPU', [sum('cluster.node_count'), sum('cluster.autoscaling.recommended_node_count_for_cpu')], { alias: ['nodes', 'recommended'], w: 4 }),
          ]),
          g('Server requests', [
            ts('Requests by method', sum('server.request_count', 'method'), { display: 'bars', w: 4 }),
            ts('Errors by code', sum('server.error_count', 'error_code'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Server latency by method', avg('server.latencies.avg', 'method'), { w: 4 }),
            ts('Rows modified and returned', [sum('server.modified_rows_count'), sum('server.returned_rows_count')], { alias: ['modified', 'returned'] }),
            top('Busiest tables', sum('server.request_count', 'table')),
          ]),
          g('Table and disk storage', [
            top('Largest tables', max('table.bytes_used', 'table'), { agg: 'last', w: 4 }),
            ts('Storage used of capacity, by cluster', max('cluster.storage_utilization', 'cluster'), { w: 4 }),
            ts('Disk used and capacity', [sum('disk.bytes_used'), sum('disk.storage_capacity')], { alias: ['used', 'capacity'], w: 4 }),
          ]),
          g('Replication and the client view', [
            ts('Replication latency', avg('replication.latency.avg', 'cluster'), { w: 4 }),
            ts('Client operation latency', avg('client.operation_latencies.avg', 'method'), { w: 4 }),
            ts('Client retries and connectivity errors', [sum('client.retry_count'), sum('client.connectivity_error_count')], { alias: ['retries', 'connectivity errors'], palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },

  'bigquery.googleapis.com/': {
    vars: ['location'],
    labels: ['priority', 'reservation', 'job_type', 'dataset_id', 'table'],
    monitor: 'google_cloud_bigquery',
    note: 'BigQuery projects: queries in flight and how long they run, bytes scanned against bytes billed, slots allocated per reservation and job type against committed capacity, and stored and loaded data per dataset.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Queries in flight', sum('query.count'), { agg: 'last' }),
          qv('Queries run', sum('query.execution_count')),
          qv('Bytes billed', sum('query.statement_scanned_bytes_billed')),
          qv('Slots allocated', sum('slots.allocated')),
          qv('Stored', sum('storage.stored_bytes'), { agg: 'last' }),
        ],
        groups: [
          g('Queries and what they cost', [
            ts('Queries in flight by priority', sum('query.count', 'priority'), { w: 4 }),
            ts('Execution time by priority', avg('query.execution_times.avg', 'priority'), { w: 4 }),
            ts('Bytes scanned and billed', [sum('query.statement_scanned_bytes'), sum('query.statement_scanned_bytes_billed')], { alias: ['scanned', 'billed'], display: 'bars', w: 4 }),
          ]),
          g('Slots and reservations', [
            ts('Slots allocated by reservation', sum('slots.allocated', 'reservation'), { w: 4 }),
            ts('Slots by job type', sum('slots.allocated', 'job_type'), { w: 4 }),
            ts('Assigned against committed capacity', [sum('slots.assigned'), sum('slots.capacity_committed')], { alias: ['assigned', 'committed'], w: 4 }),
            ts('Jobs in flight by type', sum('job.num_in_flight', 'job_type'), { display: 'bars', w: 12 }),
          ]),
          g('Datasets and loading', [
            top('Largest datasets', sum('storage.stored_bytes', 'dataset_id'), { agg: 'last', w: 4 }),
            ts('Tables by dataset', sum('storage.table_count', 'dataset_id'), { w: 4 }),
            ts('Bytes uploaded by dataset', sum('storage.uploaded_bytes', 'dataset_id'), { display: 'bars', w: 4 }),
            ts('Rows streamed (insertAll)', sum('storage.insertall_inserted_rows'), { display: 'bars' }),
            ts('Rows uploaded by dataset', sum('storage.uploaded_row_count', 'dataset_id'), { display: 'bars' }),
          ]),
        ],
      };
    },
  },

  'firestore.googleapis.com/': {
    vars: ['location', 'database_id'],
    labels: ['database_id', 'api_method', 'response_code'],
    monitor: 'google_cloud_firestore',
    note: 'Firestore databases: document reads, writes and deletes, API calls by method and response code with their latency, the billable units they add up to, realtime listeners and connections, and storage, backups and TTL deletions.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Document reads', sum('document.read_ops_count')),
          qv('Document writes', sum('document.write_ops_count')),
          qv('Document deletes', sum('document.delete_ops_count')),
          qv('Active connections', sum('network.active_connections'), { agg: 'last' }),
          qv('Data and indexes', sum('storage.data_and_index_storage_bytes'), { agg: 'last' }),
        ],
        groups: [
          g('Document operations', [
            ts('Reads, writes and deletes', [sum('document.read_ops_count'), sum('document.write_ops_count'), sum('document.delete_ops_count')], { alias: ['reads', 'writes', 'deletes'], w: 8 }),
            top('Busiest databases', sum('document.read_ops_count', 'database_id'), { w: 4 }),
          ]),
          g('API calls', [
            ts('Requests by method', sum('api.request_count', 'api_method'), { display: 'bars', w: 4 }),
            ts('Requests by response code', sum('api.request_count', 'response_code'), { display: 'bars', w: 4 }),
            ts('Latency by method', avg('api.request_latencies.avg', 'api_method'), { w: 4 }),
          ]),
          g('Billable units and listeners', [
            ts('Billable read and write units', [sum('api.billable_read_units'), sum('api.billable_write_units'), sum('api.billable_realtime_read_units')], {
              alias: ['reads', 'writes', 'realtime reads'],
              display: 'bars',
              w: 6,
            }),
            ts('Realtime listeners and connections', [sum('network.snapshot_listeners'), sum('network.active_connections')], { alias: ['listeners', 'connections'], w: 6 }),
          ]),
          g('Storage, backups and TTL', [
            ts('Data and index storage', max('storage.data_and_index_storage_bytes', 'database_id'), { w: 4 }),
            ts('Backups and point-in-time recovery', [sum('storage.backups_storage_bytes'), sum('storage.pitr_storage_bytes')], { alias: ['backups', 'PITR'], w: 4 }),
            ts('TTL deletions and their delay', [sum('document.ttl_deletion_count'), avg('document.ttl_expiration_to_deletion_delays.avg')], { alias: ['deleted', 'delay'], w: 4 }),
          ]),
        ],
      };
    },
  },

  'datastore.googleapis.com/': {
    vars: ['module_id', 'version_id'],
    labels: ['module_id', 'version_id', 'api_method', 'response_code'],
    monitor: 'google_cloud_datastore',
    note: 'Datastore (Firestore in Datastore mode) as App Engine sees it: requests by method and response code, the size of entities read and written, index writes and TTL deletions.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Requests', sum('api.request_count')),
          qv('Unsuccessful requests', sum('api.request_count', null, '!response_code:success'), { bad: true }),
          qv('Index writes', sum('index.write_count')),
          qv('TTL deletions', sum('entity.ttl_deletion_count')),
        ],
        groups: [
          g('Requests by method and outcome', [
            ts('Requests by method', sum('api.request_count', 'api_method'), { display: 'bars', w: 4 }),
            ts('Requests by response code', sum('api.request_count', 'response_code'), { display: 'bars', w: 4 }),
            top('Busiest services', sum('api.request_count', 'module_id'), { w: 4 }),
          ]),
          g('Entities and indexes', [
            ts('Entity size read and written', [avg('entity.read_sizes.avg'), avg('entity.write_sizes.avg')], { alias: ['read', 'written'], w: 4 }),
            ts('Index writes', sum('index.write_count', 'module_id'), { display: 'bars', w: 4 }),
            ts('Expiry to TTL deletion', avg('entity.ttl_expiration_to_deletion_delays.avg'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'redis.googleapis.com/': {
    vars: ['region', 'instance_id', 'cluster_id'],
    labels: ['instance_id', 'node_id', 'cluster_id', 'shard_id', 'cmd', 'role'],
    monitor: 'google_cloud_redis',
    note: 'Memorystore for Redis, standalone instances and clusters: memory used against maxmemory and evictions, clients, commands, the cache hit ratio, and for clusters each node’s CPU and memory, failed calls and replication.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Fullest instance memory', max('stats.memory.usage_ratio'), { precision: 2 }),
          qv('Connected clients', sum('clients.connected'), { agg: 'last' }),
          qv('Cache hit ratio', avg('stats.cache_hit_ratio'), { precision: 2 }),
          qv('Evicted keys', sum('stats.evicted_keys'), { bad: true }),
          qv('Busiest cluster CPU', max('cluster.cpu.maximum_utilization'), { precision: 2 }),
        ],
        groups: [
          g('Instances: memory and evictions', [
            ts('Memory used of maxmemory', max('stats.memory.usage_ratio', 'instance_id'), { w: 4 }),
            ts('System memory used', max('stats.memory.system_memory_usage_ratio', 'instance_id'), { w: 4 }),
            ts('Keys evicted and expired', [sum('stats.evicted_keys'), sum('stats.expired_keys')], { alias: ['evicted', 'expired'], palette: 'warm', w: 4 }),
          ]),
          g('Instances: clients, commands and keys', [
            ts('Clients by instance', sum('clients.connected', 'instance_id'), { w: 4 }),
            top('Busiest commands', sum('commands.calls', 'cmd'), { w: 4 }),
            ts('Cache hit ratio by instance', min('stats.cache_hit_ratio', 'instance_id'), { w: 4 }),
            ts('Keys by instance', sum('keyspace.keys', 'instance_id')),
            ts('Rejected connections', sum('stats.reject_connections_count', 'instance_id'), { display: 'bars', palette: 'warm' }),
          ]),
          g('Clusters: nodes', [
            ts('Node CPU by shard', max('cluster.node.cpu.utilization', 'shard_id'), { w: 4 }),
            ts('Node memory by shard', max('cluster.node.memory.utilization', 'shard_id'), { w: 4 }),
            ts('Clients by cluster', sum('cluster.node.clients.connected_clients', 'cluster_id'), { w: 4 }),
          ]),
          g('Clusters: calls, keyspace and replication', [
            ts('Calls and failed calls', [sum('cluster.node.commandstats.calls_count'), sum('cluster.node.commandstats.failed_calls_count')], { alias: ['calls', 'failed'], w: 4 }),
            ts('Keyspace hit share', [sum('cluster.node.stats.keyspace_hits_count', 'cluster_id'), sum('cluster.node.stats.keyspace_misses_count', 'cluster_id')], {
              formulas: [['100 * query1 / (query1 + query2)', 'hit %']],
              w: 4,
            }),
            ts('Replica acknowledgement lag', max('cluster.replication.maximum_ack_lag', 'cluster_id'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'memorystore.googleapis.com/': {
    vars: ['location', 'instance_id', 'shard_id'],
    labels: ['instance_id', 'shard_id', 'node_id', 'cmd', 'role'],
    monitor: 'google_cloud_memorystore',
    note: 'Memorystore for Valkey instances, shard by shard: node CPU and memory, commands and the errors they return, keyspace hits and evictions, replication and persistence, and clients.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Busiest instance CPU', max('instance.cpu.maximum_utilization'), { precision: 2 }),
          qv('Fullest instance memory', max('instance.memory.maximum_utilization'), { precision: 2 }),
          qv('Clients', sum('instance.clients.total_connected_clients'), { agg: 'last' }),
          qv('Keyspace hit %', [sum('instance.node.stats.keyspace_hits_count'), sum('instance.node.stats.keyspace_misses_count')], { formula: '100 * query1 / (query1 + query2)', precision: 1 }),
          qv('Command errors', sum('instance.node.errorstats.errors_count'), { bad: true }),
        ],
        groups: [
          g('Shards and nodes', [
            ts('Node CPU by shard', max('instance.node.cpu.utilization', 'shard_id'), { w: 4 }),
            ts('Node memory by shard', max('instance.node.memory.utilization', 'shard_id'), { w: 4 }),
            top('Fullest nodes', max('instance.node.memory.utilization', 'node_id'), { agg: 'avg', w: 4 }),
          ]),
          g('Commands and errors', [
            top('Busiest commands', sum('instance.node.commandstats.calls_count', 'cmd'), { w: 4 }),
            ts('Failed and rejected calls', [sum('instance.node.commandstats.failed_calls_count'), sum('instance.node.commandstats.rejected_calls_count')], {
              alias: ['failed', 'rejected'],
              palette: 'warm',
              w: 4,
            }),
            ts('Time spent in commands', sum('instance.node.commandstats.usec_count', 'cmd'), { display: 'bars', w: 4 }),
          ]),
          g('Keyspace and evictions', [
            ts('Keyspace hits and misses', [sum('instance.node.stats.keyspace_hits_count'), sum('instance.node.stats.keyspace_misses_count')], { alias: ['hits', 'misses'], w: 4 }),
            ts('Keys evicted and expired', [sum('instance.node.stats.evicted_keys_count'), sum('instance.node.stats.expired_keys_count')], { alias: ['evicted', 'expired'], palette: 'warm', w: 4 }),
            ts('Keys by instance', sum('instance.keyspace.total_keys', 'instance_id'), { w: 4 }),
          ]),
          g('Replication and persistence', [
            ts('Replica acknowledgement lag', max('instance.replication.maximum_ack_lag', 'instance_id'), { w: 4 }),
            ts('Full resyncs', sum('instance.node.replication.sync_full_count', 'shard_id'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('AOF fsync lag', max('instance.node.persistence.aof_fsync_lag', 'node_id'), { w: 4 }),
          ]),
          g('Clients and network', [
            ts('Connected and blocked clients', [sum('instance.node.clients.connected_clients'), sum('instance.node.clients.blocked_clients')], { alias: ['connected', 'blocked'], w: 6 }),
            mirror('Network in and out', sum('instance.node.stats.net_input_bytes_count'), sum('instance.node.stats.net_output_bytes_count'), { w: 6 }),
          ]),
        ],
      };
    },
  },

  'pubsub.googleapis.com/': {
    vars: ['topic_id', 'subscription_id'],
    labels: ['topic_id', 'subscription_id', 'delivery_type', 'response_class', 'response_code'],
    monitor: 'google_cloud_pubsub',
    note: 'Pub/Sub topics and subscriptions: publish requests and their latency, messages delivered by pull or push and acknowledged, the backlog of undelivered messages and the age of the oldest unacked one, expired deadlines and dead letters.',
    logs: [['Pub/Sub logs', 'source:gcp.pubsub.subscription']],
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Publish requests', sum('topic.send_request_count')),
          qv('Messages delivered', sum('subscription.sent_message_count')),
          qv('Undelivered', sum('subscription.num_undelivered_messages'), { agg: 'last' }),
          qv('Oldest unacked message', max('subscription.oldest_unacked_message_age'), { agg: 'max' }),
          qv('Dead-lettered', sum('subscription.dead_letter_message_count'), { bad: true }),
          qv('Expired ack deadlines', sum('subscription.expired_ack_deadlines_count'), { bad: true }),
        ],
        groups: [
          g('Publishing to topics', [
            ts('Publish requests by topic', sum('topic.send_request_count', 'topic_id'), { display: 'bars', w: 4 }),
            ts('Failed publishes by code', sum('topic.send_request_count', 'response_code', '!response_class:success'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Publish latency', avg('topic.send_request_latencies.avg', 'topic_id'), { w: 4 }),
            ts('Message size', avg('topic.message_sizes.avg', 'topic_id')),
            ts('Retained messages', sum('topic.num_retained_messages', 'topic_id')),
          ]),
          g('Delivery to subscribers', [
            ts('Delivered by pull and push', sum('subscription.sent_message_count', 'delivery_type'), { display: 'bars', w: 4 }),
            ts('Delivered and acknowledged', [sum('subscription.sent_message_count'), sum('subscription.ack_message_count')], { alias: ['delivered', 'acked'], w: 4 }),
            ts('Push requests by outcome', sum('subscription.push_request_count', 'response_class'), { display: 'bars', w: 4 }),
            ts('Push latency', avg('subscription.push_request_latencies.avg', 'subscription_id')),
            ts('Open streaming pulls', sum('subscription.open_streaming_pulls', 'subscription_id')),
          ]),
          g('Backlog', [
            top('Largest backlogs', sum('subscription.num_undelivered_messages', 'subscription_id'), { agg: 'last', w: 4, h: 4 }),
            ts('Undelivered messages by subscription', sum('subscription.num_undelivered_messages', 'subscription_id'), { w: 8 }),
            ts('Oldest unacked message age', max('subscription.oldest_unacked_message_age', 'subscription_id'), { w: 4 }),
            ts('Backlog size', sum('subscription.backlog_bytes', 'subscription_id'), { w: 4 }),
          ]),
          g('Acknowledgements and dead letters', [
            ts('Ack latency', avg('subscription.ack_latencies.avg', 'subscription_id'), { w: 4 }),
            ts('Expired ack deadlines', sum('subscription.expired_ack_deadlines_count', 'subscription_id'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Dead-lettered messages', sum('subscription.dead_letter_message_count', 'subscription_id'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },

  'storage.googleapis.com/': {
    vars: ['location', 'bucket_name', 'storage_class'],
    labels: ['bucket_name', 'storage_class', 'method', 'response_code'],
    monitor: 'google_cloud_storage',
    note: 'Cloud Storage buckets: what they store by storage class (sampled once a day, so over a week), requests by method and response code, bytes sent and received, and Autoclass transitions and dual-region replication.',
    logs: [['Cloud Storage logs', 'source:gcp.gcs.bucket']],
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Stored', sum('storage.total_bytes'), { agg: 'last', span: '1w' }),
          qv('Objects', sum('storage.object_count'), { agg: 'last', span: '1w' }),
          qv('Requests', sum('api.request_count')),
          qv('Unsuccessful requests', sum('api.request_count', null, '!response_code:OK'), { bad: true }),
          qv('Sent', sum('network.sent_bytes_count')),
        ],
        groups: [
          g('Stored data', [
            ts('Stored by bucket', max('storage.total_bytes', 'bucket_name'), { w: 4, span: '1w' }),
            tree('Stored by storage class', sum('storage.total_bytes', 'storage_class'), { agg: 'last', w: 4, span: '1w' }),
            chg('Bucket size, week over week', `max:gcp.${q.short}.storage.total_bytes{${q.f}} by {bucket_name}`, { w: 4 }),
            ts('Objects by bucket', max('storage.object_count', 'bucket_name'), { span: '1w' }),
            ts('Bytes deleted', sum('storage.v2.deleted_bytes', 'bucket_name'), { display: 'bars', span: '1w' }),
          ]),
          g('API requests', [
            ts('Requests by method', sum('api.request_count', 'method'), { display: 'bars', w: 4 }),
            ts('Unsuccessful requests by code', sum('api.request_count', 'response_code', '!response_code:OK'), { display: 'bars', palette: 'warm', w: 4 }),
            top('Busiest buckets', sum('api.request_count', 'bucket_name'), { w: 4 }),
          ]),
          g('Egress and ingress', [
            mirror('Bytes received and sent', sum('network.received_bytes_count'), sum('network.sent_bytes_count'), { w: 8 }),
            top('Most egress', sum('network.sent_bytes_count', 'bucket_name'), { w: 4 }),
          ]),
          g('Autoclass and replication', [
            ts('Autoclass transitions', sum('autoclass.transition_operation_count', 'bucket_name'), { display: 'bars', w: 4 }),
            ts('Turbo replication delay', max('replication.turbo_max_delay', 'bucket_name'), { w: 4 }),
            top('Minutes missing the RPO (30 days)', max('replication.missing_rpo_minutes_last_30d', 'bucket_name'), { agg: 'last', w: 4 }),
          ]),
        ],
      };
    },
  },

  'loadbalancing.googleapis.com/': {
    vars: ['url_map_name', 'forwarding_rule_name', 'backend_target_name'],
    labels: [
      'url_map_name',
      'forwarding_rule_name',
      'target_proxy_name',
      'backend_target_name',
      'backend_name',
      'response_code',
      'response_code_class',
      'cache_result',
      'proxy_continent',
      'client_country',
    ],
    monitor: 'google_cloud_loadbalancing',
    note: 'Cloud Load Balancing: the external Application Load Balancer’s requests by response class, total and backend latency percentiles, backends and Cloud CDN cache results; internal and regional HTTP(S), the TCP/SSL proxy and network (pass-through) load balancers.',
    logs: [['Load balancer logs', 'source:gcp.http.load.balancer']],
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Requests', sum('https.request_count')),
          qv('5xx responses', sum('https.request_count', null, 'response_code_class:500'), { bad: true }),
          qv('p95 total latency', max('https.total_latencies.p95'), { precision: 0 }),
          qv('p99 backend latency', max('https.backend_latencies.p99'), { precision: 0 }),
          qv('Served from CDN cache', sum('https.request_count', null, 'cache_result:HIT')),
          qv('Response bytes', sum('https.response_bytes_count')),
        ],
        groups: [
          g('External HTTP(S) requests', [
            ts('Requests by response class', sum('https.request_count', 'response_code_class'), { display: 'bars', w: 8 }),
            top('Busiest URL maps', sum('https.request_count', 'url_map_name'), { w: 4 }),
            share('5xx share by URL map', sum('https.request_count', 'url_map_name', 'response_code_class:500'), sum('https.request_count', 'url_map_name')),
            pie('Requests by client continent', sum('https.request_count', 'proxy_continent'), { w: 6 }),
          ]),
          g('Latency at the frontend and the backend', [
            tail('Total latency', on(q), 'https.total_latencies', { w: 4 }),
            tail('Backend latency', on(q), 'https.backend_latencies', { w: 4 }),
            ts('Frontend TCP round trip (p95)', max('https.frontend_tcp_rtt.p95', 'proxy_continent'), { w: 4 }),
            top('Slowest URL maps (p99)', max('https.total_latencies.p99', 'url_map_name'), { agg: 'avg', w: 6 }),
            ts('Average total latency by URL map', avg('https.total_latencies.avg', 'url_map_name'), { w: 6 }),
          ]),
          g('Backends', [
            ts('Backend requests by backend service', sum('https.backend_request_count', 'backend_target_name'), { display: 'bars', w: 4 }),
            ts('Backend latency p95 by backend service', max('https.backend_latencies.p95', 'backend_target_name'), { w: 4 }),
            mirror('Bytes to and from backends', sum('https.backend_request_bytes_count'), sum('https.backend_response_bytes_count'), { w: 4 }),
          ]),
          g('Cloud CDN', [
            ts('Requests by cache result', sum('https.request_count', 'cache_result'), { display: 'bars', w: 4 }),
            share('Cache hit share by URL map', sum('https.request_count', 'url_map_name', 'cache_result:HIT'), sum('https.request_count', 'url_map_name'), { w: 4 }),
            ts('Response bytes by cache result', sum('https.response_bytes_count', 'cache_result'), { display: 'bars', w: 4 }),
          ]),
          g('Internal and regional HTTP(S)', [
            ts('Internal requests by response class', sum('https.internal.request_count', 'response_code_class'), { display: 'bars', w: 4 }),
            tail('Internal total latency', on(q), 'https.internal.total_latencies', { w: 4 }),
            ts('Regional external requests', sum('https.external.regional.request_count', 'response_code_class'), { display: 'bars', w: 4 }),
          ]),
          g('TCP/SSL proxy', [
            ts('Connections opened and closed', [sum('tcp_ssl_proxy.new_connections'), sum('tcp_ssl_proxy.closed_connections')], { alias: ['opened', 'closed'], w: 4 }),
            ts('Open connections', sum('tcp_ssl_proxy.open_connections', 'forwarding_rule_name'), { w: 4 }),
            ts('Frontend round trip (p95)', max('tcp_ssl_proxy.frontend_tcp_rtt.p95', 'forwarding_rule_name'), { w: 4 }),
            mirror('Bytes in and out', sum('tcp_ssl_proxy.ingress_bytes_count'), sum('tcp_ssl_proxy.egress_bytes_count'), { w: 12 }),
          ]),
          g('Network load balancers', [
            mirror('External: bytes in and out', sum('l3.external.ingress_bytes_count'), sum('l3.external.egress_bytes_count'), { w: 4 }),
            ts('External: round trip (p95)', max('l3.external.rtt_latencies.p95', 'forwarding_rule_name'), { w: 4 }),
            mirror('Internal: bytes in and out', sum('l3.internal.ingress_bytes_count'), sum('l3.internal.egress_bytes_count'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'edgecache.googleapis.com/': {
    vars: ['location', 'service_name'],
    labels: ['service_name', 'response_code_class', 'cache_result'],
    monitor: 'google_cloud_media_cdn',
    note: 'Media CDN services: edge requests by cache result and response class, how much traffic the edge keeps from the origin, time to first and last byte at the edge and from the origin, and players’ buffering (CMCD).',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Edge requests', sum('edge_cache_route_rule.request_count')),
          qv('Origin requests', sum('edge_cache_route_rule.origin_request_count')),
          qv('Origin offload %', [sum('edge_cache_route_rule.origin_request_count'), sum('edge_cache_route_rule.request_count')], { formula: '100 - 100 * query1 / query2', precision: 1 }),
          qv('Egress', sum('edge_cache_route_rule.response_bytes_count')),
          qv('Avg time to first byte', avg('edge_cache_route_rule.http_ttfb.avg'), { precision: 0 }),
        ],
        groups: [
          g('Edge requests and cache results', [
            ts('Requests by cache result', sum('edge_cache_route_rule.request_count', 'cache_result'), { display: 'bars', w: 4 }),
            ts('Requests by response class', sum('edge_cache_route_rule.request_count', 'response_code_class'), { display: 'bars', w: 4 }),
            top('Busiest services', sum('edge_cache_route_rule.request_count', 'service_name'), { w: 4 }),
          ]),
          g('Origin offload', [
            ts('Edge against origin requests', [sum('edge_cache_route_rule.request_count'), sum('edge_cache_route_rule.origin_request_count')], { alias: ['edge', 'origin'], w: 6 }),
            ts('Bytes from the edge and from the origin', [sum('edge_cache_route_rule.response_bytes_count'), sum('edge_cache_route_rule.origin_response_bytes_count')], {
              alias: ['edge', 'origin'],
              display: 'bars',
              w: 6,
            }),
          ]),
          g('Time to first and last byte', [
            ts('At the edge', [avg('edge_cache_route_rule.http_ttfb.avg'), avg('edge_cache_route_rule.http_ttlb.avg')], { alias: ['first byte', 'last byte'], w: 4 }),
            ts('From the origin', [avg('edge_cache_route_rule.origin_http_ttfb.avg'), avg('edge_cache_route_rule.origin_http_ttlb.avg')], { alias: ['first byte', 'last byte'], w: 4 }),
            ts('Total latency by service', avg('edge_cache_route_rule.total_latencies.avg', 'service_name'), { w: 4 }),
          ]),
          g('Video playback (CMCD)', [
            ts('Players reporting', sum('edge_cache_route_rule.cmcd.request_count', 'service_name'), { display: 'bars', w: 4 }),
            ts('Buffer starvation', sum('edge_cache_route_rule.cmcd.buffer_starvation_count', 'service_name'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Buffer length', avg('edge_cache_route_rule.cmcd.buffer_lengths.avg', 'service_name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'dns.googleapis.com/': {
    vars: ['target_type', 'target_name'],
    labels: ['target_type', 'target_name', 'source_type', 'response_code'],
    monitor: 'google_cloud_dns',
    note: 'Cloud DNS queries answered for private zones and policies: volume by zone and source, response codes (NXDOMAIN, SERVFAIL, …) and how long answers take.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Queries', sum('query.response_count')),
          qv('NXDOMAIN', sum('query.response_count', null, 'response_code:NXDOMAIN')),
          qv('SERVFAIL', sum('query.response_count', null, 'response_code:SERVFAIL'), { bad: true }),
          qv('Avg latency', avg('query.latencies.avg'), { precision: 1 }),
        ],
        groups: [
          g('Queries by zone and source', [
            ts('Queries by zone or policy', sum('query.response_count', 'target_name'), { display: 'bars', w: 8 }),
            pie('By source', sum('query.response_count', 'source_type')),
          ]),
          g('Answers and their latency', [
            ts('Responses by code', sum('query.response_count', 'response_code'), { display: 'bars', w: 4 }),
            ts('Latency by zone', avg('query.latencies.avg', 'target_name'), { w: 4 }),
            top('Slowest zones', max('query.latencies.avg', 'target_name'), { agg: 'avg', w: 4 }),
          ]),
        ],
      };
    },
  },

  'router.googleapis.com/': {
    vars: ['region', 'router_id', 'gateway_name'],
    labels: ['router_id', 'gateway_name', 'bgp_peer_name', 'nat_ip', 'ip_protocol', 'reason'],
    monitor: 'google_cloud_router',
    note: 'Cloud Routers and the Cloud NAT gateways they run: BGP and BFD sessions, routes learned and advertised against the limits, NAT ports and connections, NAT traffic and the packets it drops (and why).',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Routers up', sum('router_up'), { agg: 'last', good: true }),
          qv('BGP sessions down', sum('bgp_sessions_down_count'), { agg: 'last', bad: true }),
          qv('NAT open connections', sum('nat.open_connections'), { agg: 'last' }),
          qv('NAT dropped packets', sum('nat.dropped_sent_packets_count'), { bad: true }),
          qv('NAT ports allocated', sum('nat.allocated_ports'), { agg: 'last' }),
        ],
        groups: [
          g('BGP and BFD sessions', [
            ts('Sessions up and down', [sum('bgp_sessions_up_count'), sum('bgp_sessions_down_count')], { alias: ['up', 'down'], w: 4 }),
            ts('Session up by peer', min('bgp.session_up', 'bgp_peer_name'), { w: 4 }),
            ts('BFD session flaps', sum('bfd.session_flap_events_count', 'bgp_peer_name'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
          g('Routes', [
            ts('Routes received by peer', max('bgp.received_routes_count', 'bgp_peer_name'), { w: 4 }),
            ts('Routes advertised by peer', max('bgp.sent_routes_count', 'bgp_peer_name'), { w: 4 }),
            ts('Learned destinations against the limit', [max('dynamic_routes.learned_routes.used_unique_destinations'), max('dynamic_routes.learned_routes.unique_destinations_limit')], {
              alias: ['used', 'limit'],
              w: 4,
            }),
          ]),
          g('Cloud NAT ports and connections', [
            ts('Ports allocated by NAT IP', sum('nat.allocated_ports', 'nat_ip'), { w: 4 }),
            ts('Highest port usage per VM', max('nat.port_usage', 'gateway_name'), { w: 4 }),
            ts('Connections opened and closed', [sum('nat.new_connections_count'), sum('nat.closed_connections_count')], { alias: ['opened', 'closed'], w: 4 }),
          ]),
          g('Cloud NAT traffic and drops', [
            mirror('NAT traffic in and out', sum('nat.received_bytes_count'), sum('nat.sent_bytes_count'), { w: 6 }),
            ts('Sent packets dropped, by reason', sum('nat.dropped_sent_packets_count', 'reason'), { display: 'bars', palette: 'warm', w: 6 }),
          ]),
        ],
      };
    },
  },

  'vpn.googleapis.com/': {
    vars: ['region', 'gateway_id', 'tunnel_name'],
    labels: ['gateway_id', 'tunnel_name'],
    monitor: 'google_cloud_vpn',
    note: 'Cloud VPN gateways and their tunnels: which tunnels are established, traffic through each and the packets dropped on the way.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Tunnels established', sum('tunnel_established'), { agg: 'last', good: true }),
          qv('Gateway connections', sum('gateway.connections'), { agg: 'last' }),
          qv('Received', sum('network.received_bytes_count')),
          qv('Sent', sum('network.sent_bytes_count')),
          qv('Dropped packets', [sum('network.dropped_received_packets_count'), sum('network.dropped_sent_packets_count')], { formula: 'query1 + query2', bad: true }),
        ],
        groups: [
          g('Tunnel state', [ts('Established by tunnel', min('tunnel_established', 'tunnel_name'), { w: 6 }), ts('Connections by gateway', sum('gateway.connections', 'gateway_id'), { w: 6 })]),
          g('Traffic through tunnels', [
            mirror('Bytes in and out', sum('network.received_bytes_count'), sum('network.sent_bytes_count'), { w: 4 }),
            ts('Packets in and out', [sum('network.received_packets_count'), sum('network.sent_packets_count')], { alias: ['in', 'out'], w: 4 }),
            top('Busiest tunnels', sum('network.sent_bytes_count', 'tunnel_name'), { w: 4 }),
          ]),
          g('Dropped packets', [
            ts('Dropped on receipt, by tunnel', sum('network.dropped_received_packets_count', 'tunnel_name'), { display: 'bars', palette: 'warm' }),
            ts('Dropped on sending, by tunnel', sum('network.dropped_sent_packets_count', 'tunnel_name'), { display: 'bars', palette: 'warm' }),
          ]),
        ],
      };
    },
  },

  'interconnect.googleapis.com/': {
    vars: ['interconnect', 'attachment'],
    labels: ['interconnect', 'attachment', 'link_id'],
    monitor: 'google_cloud_interconnect',
    note: 'Dedicated and Partner Interconnect: whether circuits and their links are up and the light levels they receive, traffic, errors and drops per circuit, VLAN attachments and MACsec.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Circuits operational', sum('network.interconnect.operational'), { agg: 'last', good: true }),
          qv('Capacity', sum('network.interconnect.capacity'), { agg: 'last' }),
          qv('Received', sum('network.interconnect.received_bytes_count')),
          qv('Sent', sum('network.interconnect.sent_bytes_count')),
          qv('Errors', [sum('network.interconnect.receive_errors_count'), sum('network.interconnect.send_errors_count')], { formula: 'query1 + query2', bad: true }),
        ],
        groups: [
          g('Circuits and links', [
            ts('Circuit operational', min('network.interconnect.operational', 'interconnect'), { w: 4 }),
            ts('Link operational', min('network.interconnect.link.operational', 'link_id'), { w: 4 }),
            ts('Light received and sent (dBm)', [min('network.interconnect.link.rx_power'), min('network.interconnect.link.tx_power')], { alias: ['rx', 'tx'], w: 4 }),
          ]),
          g('Circuit traffic, errors and drops', [
            mirror('Bytes in and out', sum('network.interconnect.received_bytes_count'), sum('network.interconnect.sent_bytes_count'), { w: 4 }),
            ts('Receive and send errors', [sum('network.interconnect.receive_errors_count'), sum('network.interconnect.send_errors_count')], { alias: ['receive', 'send'], palette: 'warm', w: 4 }),
            ts('Dropped packets by circuit', sum('network.interconnect.dropped_packets_count', 'interconnect'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
          g('VLAN attachments', [
            mirror('Attachment bytes in and out', sum('network.attachment.received_bytes_count'), sum('network.attachment.sent_bytes_count'), { w: 4 }),
            top('Busiest attachments', sum('network.attachment.sent_bytes_count', 'attachment'), { w: 4 }),
            ts('Attachment drops', [sum('network.attachment.ingress_dropped_packets_count'), sum('network.attachment.egress_dropped_packets_count')], {
              alias: ['ingress', 'egress'],
              palette: 'warm',
              w: 4,
            }),
          ]),
          g('MACsec', [
            ts('MACsec operational by link', min('network.interconnect.link.macsec.operational', 'link_id'), { w: 6 }),
            ts('MACsec errors', [sum('network.interconnect.link.macsec.receive_errors_count'), sum('network.interconnect.link.macsec.send_errors_count')], {
              alias: ['receive', 'send'],
              palette: 'warm',
              w: 6,
            }),
          ]),
          g('Capacity', [
            txt('Each circuit’s capacity is in bytes per second; compare it with the traffic above to see how full a circuit runs.', { w: 4 }),
            ts('Capacity by circuit', max('network.interconnect.capacity', 'interconnect'), { w: 8 }),
          ]),
        ],
      };
    },
  },

  'networksecurity.googleapis.com/': {
    vars: ['location', 'policy_name'],
    labels: ['policy_name', 'blocked', 'backend_target_name'],
    monitor: 'google_cloud_armor',
    note: 'Cloud Armor security policies: HTTP(S) requests they allow and block (and would block in preview mode), enforcement on proxy and network load balancers, DDoS traffic, and Cloud NGFW endpoints.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Requests evaluated', sum('https.request_count')),
          qv('Requests blocked', sum('https.request_count', null, 'blocked:true'), { bad: true }),
          qv('Would block (preview)', sum('https.previewed_request_count', null, 'blocked:true')),
          qv('DDoS packets', sum('dos.ingress_packets_count')),
          qv('NGFW threats', sum('firewall_endpoint.threats_count'), { bad: true }),
        ],
        groups: [
          g('HTTP(S) requests by verdict', [
            ts('Allowed and blocked', sum('https.request_count', 'blocked'), { display: 'bars', w: 4 }),
            top('Policies blocking the most', sum('https.request_count', 'policy_name', 'blocked:true'), { palette: 'warm', w: 4 }),
            top('Backends blocked the most', sum('https.request_count', 'backend_target_name', 'blocked:true'), { palette: 'warm', w: 4 }),
            ts('Preview mode: would block', sum('https.previewed_request_count', 'policy_name', 'blocked:true'), { display: 'bars', w: 12 }),
          ]),
          g('Proxy and network enforcement', [
            ts('TCP/SSL proxy connections by verdict', sum('tcp_ssl_proxy.new_connection_count', 'blocked'), { display: 'bars' }),
            ts('Network load balancer packets by verdict', sum('l3.external.packet_count', 'blocked'), { display: 'bars' }),
          ]),
          g('DDoS and NGFW endpoints', [
            ts('DDoS ingress', [sum('dos.ingress_packets_count'), sum('dos.ingress_bytes_count')], { alias: ['packets', 'bytes'], w: 4 }),
            ts('Threats detected by NGFW endpoints', sum('firewall_endpoint.threats_count'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Endpoint health', min('firewall_endpoint.health'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'file.googleapis.com/': {
    vars: ['location', 'instance_name'],
    labels: ['instance_name', 'file_share'],
    monitor: 'google_cloud_filestore',
    note: 'Filestore instances: free space per share and snapshots, NFS throughput and operations, average read and write latency, availability and replication.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Instances available', sum('nfs.server.instance_available'), { agg: 'last', good: true }),
          qv('Lowest free space %', min('nfs.server.free_bytes_percent'), { agg: 'min', precision: 1 }),
          qv('Read', sum('nfs.server.read_bytes_count')),
          qv('Written', sum('nfs.server.write_bytes_count')),
          qv('Avg write latency', avg('nfs.server.average_write_latency'), { precision: 1 }),
        ],
        groups: [
          g('Capacity by share', [
            top('Fullest shares', max('nfs.server.used_bytes_percent', 'file_share'), { agg: 'avg', w: 4 }),
            ts('Used by instance', max('nfs.server.used_bytes', 'instance_name'), { w: 4 }),
            ts('Snapshots', max('nfs.server.snapshots_used_bytes', 'instance_name'), { w: 4 }),
          ]),
          g('NFS throughput and operations', [
            ts('Bytes read and written', [sum('nfs.server.read_bytes_count'), sum('nfs.server.write_bytes_count')], { alias: ['read', 'written'], w: 4 }),
            ts('Read, write and metadata operations', [sum('nfs.server.read_ops_count'), sum('nfs.server.write_ops_count'), sum('nfs.server.metadata_ops_count')], {
              alias: ['reads', 'writes', 'metadata'],
              w: 4,
            }),
            ts('Average latency', [avg('nfs.server.average_read_latency'), avg('nfs.server.average_write_latency')], { alias: ['read', 'write'], w: 4 }),
          ]),
          g('Availability and replication', [
            ts('Available by instance', min('nfs.server.instance_available', 'instance_name'), { w: 4 }),
            ts('Locks held', sum('nfs.server.locks', 'instance_name'), { w: 4 }),
            ts('Since the last replication', max('instance.time_since_last_replication', 'instance_name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'dataflow.googleapis.com/': {
    vars: ['region', 'job_name'],
    labels: ['job_name', 'job_id', 'stage'],
    monitor: 'google_cloud_dataflow',
    note: 'Dataflow jobs: how fresh their output is (system lag, data watermark age), the backlog, elements produced, workers and vCPUs autoscaling gives them, Pub/Sub reads and writes, and the resources billed.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Failed jobs', sum('job.is_failed'), { agg: 'last', bad: true }),
          qv('Max system lag', max('job.system_lag'), { agg: 'max' }),
          qv('Max watermark age', max('job.data_watermark_age'), { agg: 'max' }),
          qv('Workers', sum('job.active_worker_instances'), { agg: 'last' }),
          qv('vCPUs', sum('job.current_num_vcpus'), { agg: 'last' }),
        ],
        groups: [
          g('Freshness and lag', [
            ts('System lag by job', max('job.system_lag', 'job_name'), { w: 4 }),
            ts('Data watermark age by job', max('job.data_watermark_age', 'job_name'), { w: 4 }),
            top('Most lagging stages', max('job.per_stage_system_lag', 'stage'), { agg: 'avg', w: 4 }),
          ]),
          g('Backlog and throughput', [
            ts('Backlog by job', sum('job.backlog_bytes', 'job_name'), { w: 4 }),
            ts('Estimated time to clear the backlog', max('job.estimated_backlog_processing_time', 'job_name'), { w: 4 }),
            ts('Elements produced by job', sum('job.elements_produced_count', 'job_name'), { display: 'bars', w: 4 }),
          ]),
          g('Workers and autoscaling', [
            ts('Workers against target', [sum('job.active_worker_instances'), sum('job.target_worker_instances')], { alias: ['active', 'target'], w: 4 }),
            ts('vCPUs by job', sum('job.current_num_vcpus', 'job_name'), { w: 4 }),
            ts('Worker utilization', avg('job.aggregated_worker_utilization', 'job_name'), { w: 4 }),
          ]),
          g('Pub/Sub reads and writes', [
            ts('Messages read and written', [sum('job.pubsub.read_count'), sum('job.pubsub.write_count')], { alias: ['read', 'written'], w: 6 }),
            ts('Pub/Sub latency', [avg('job.pubsub.read_latencies.avg'), avg('job.pubsub.write_latencies.avg')], { alias: ['read', 'write'], w: 6 }),
          ]),
          g('Billed resources', [
            tbl(
              'Jobs',
              [
                ['vCPU time', max('job.total_vcpu_time', 'job_name'), 'last'],
                ['Memory time', max('job.total_memory_usage_time', 'job_name'), 'last'],
                ['Shuffle data', max('job.total_shuffle_data_processed', 'job_name'), 'last'],
                ['Streaming data', max('job.total_streaming_data_processed', 'job_name'), 'last'],
              ],
              { h: 3 },
            ),
          ]),
        ],
      };
    },
  },

  'dataproc.googleapis.com/': {
    vars: ['region', 'cluster_name'],
    labels: ['cluster_name', 'cluster_uuid', 'job_type', 'operation_type', 'status'],
    monitor: 'google_cloud_dataproc',
    note: 'Dataproc clusters: jobs submitted, running and failed with their duration, YARN memory, cores and containers, HDFS capacity and unhealthy blocks, nodes running against expected, and cluster operations.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Running jobs', sum('cluster.job.running_count'), { agg: 'last' }),
          qv('Failed jobs', sum('cluster.job.failed_count'), { bad: true }),
          qv('YARN memory allocated %', max('cluster.yarn.allocated_memory_percentage'), { precision: 1 }),
          qv('Fullest HDFS', max('cluster.hdfs.storage_utilization'), { precision: 1 }),
          qv('Failed nodes', sum('cluster.nodes.failed_count'), { bad: true }),
        ],
        groups: [
          g('Jobs', [
            ts('Jobs submitted and failed', [sum('cluster.job.submitted_count'), sum('cluster.job.failed_count')], { alias: ['submitted', 'failed'], display: 'bars', w: 4 }),
            ts('Failures by job type', sum('cluster.job.failed_count', 'job_type'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Job duration', avg('cluster.job.duration.avg', 'job_type'), { w: 4 }),
          ]),
          g('YARN', [
            ts('Memory allocated by cluster', max('cluster.yarn.allocated_memory_percentage', 'cluster_name'), { w: 4 }),
            ts('Pending memory', sum('cluster.yarn.pending_memory_size', 'cluster_name'), { w: 4 }),
            ts('Containers by status', sum('cluster.yarn.containers', 'status'), { display: 'bars', w: 4 }),
            ts('Applications by status', sum('cluster.yarn.apps', 'status'), { display: 'bars' }),
            ts('Virtual cores by status', sum('cluster.yarn.virtual_cores', 'status')),
          ]),
          g('HDFS', [
            ts('Storage used by cluster', max('cluster.hdfs.storage_utilization', 'cluster_name'), { w: 4 }),
            ts('Unhealthy blocks by status', sum('cluster.hdfs.unhealthy_blocks', 'status'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('DataNodes by status', sum('cluster.hdfs.datanodes', 'status'), { w: 4 }),
          ]),
          g('Nodes and operations', [
            ts('Nodes running against expected', [sum('cluster.nodes.running'), sum('cluster.nodes.expected')], { alias: ['running', 'expected'], w: 4 }),
            ts('Nodes failed and recovered', [sum('cluster.nodes.failed_count'), sum('cluster.nodes.recovered_count')], { alias: ['failed', 'recovered'], palette: 'warm', w: 4 }),
            ts('Failed operations by type', sum('cluster.operation.failed_count', 'operation_type'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },

  'composer.googleapis.com/': {
    vars: ['location', 'environment_name'],
    labels: ['environment_name', 'image_version', 'state'],
    monitor: 'google_cloud_composer',
    note: 'Cloud Composer (Airflow) environments: health of the environment, its database and web server, DAG parsing, task instances by state and the executor’s queue, Celery workers against their bounds and the Airflow database.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Healthy environments', sum('environment.healthy'), { agg: 'last', good: true }),
          qv('Celery workers', sum('environment.num_celery_workers'), { agg: 'last' }),
          qv('Running tasks', sum('environment.executor.running_tasks'), { agg: 'last' }),
          qv('Failed tasks', sum('environment.finished_task_instance_count', null, 'state:failed'), { bad: true }),
          qv('DAG parse errors', sum('environment.dag_processing.parse_error_count'), { bad: true }),
        ],
        groups: [
          g('Environment health', [
            ts('Environment, database and web server', [min('environment.healthy'), min('environment.database_health'), min('environment.web_server.health')], {
              alias: ['environment', 'database', 'web server'],
              w: 6,
            }),
            ts('Scheduler heartbeats', sum('environment.scheduler_heartbeat_count', 'environment_name'), { display: 'bars', w: 6 }),
          ]),
          g('DAG parsing', [
            ts('DAGs', max('environment.dagbag_size', 'environment_name'), { w: 4 }),
            ts('Total parse time', max('environment.dag_processing.total_parse_time', 'environment_name'), { w: 4 }),
            ts('Parse errors and timeouts', [sum('environment.dag_processing.parse_error_count'), sum('environment.dag_processing.processor_timeout_count')], {
              alias: ['errors', 'timeouts'],
              palette: 'warm',
              w: 4,
            }),
          ]),
          g('Task instances and the executor', [
            ts('Finished tasks by state', sum('environment.finished_task_instance_count', 'state'), { display: 'bars', w: 4 }),
            ts('Running and queued', [sum('environment.executor.running_tasks'), sum('environment.executor.queued_tasks')], { alias: ['running', 'queued'], w: 4 }),
            ts('Zombie tasks killed', sum('environment.zombie_task_killed_count', 'environment_name'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
          g('Workers', [
            ts('Celery workers against their bounds', [sum('environment.num_celery_workers'), sum('environment.worker.min_workers'), sum('environment.worker.max_workers')], {
              alias: ['workers', 'min', 'max'],
              w: 6,
            }),
            ts('Worker pod evictions', sum('environment.worker.pod_eviction_count', 'environment_name'), { display: 'bars', palette: 'warm', w: 6 }),
          ]),
          g('Airflow database', [
            ts('Database CPU and memory', [max('environment.database.cpu.utilization'), max('environment.database.memory.utilization')], { alias: ['CPU', 'memory'], w: 4 }),
            ts('Database disk used', max('environment.database.disk.utilization', 'environment_name'), { w: 4 }),
            ts('Connections against the maximum', [sum('environment.database.network.connections'), sum('environment.database.network.max_connections')], { alias: ['connections', 'max'], w: 4 }),
          ]),
        ],
      };
    },
  },

  'aiplatform.googleapis.com/': {
    vars: ['location', 'model_user_id', 'endpoint_id'],
    labels: ['model_user_id', 'model_version_id', 'publisher', 'endpoint_id', 'deployed_model_id', 'response_code', 'type'],
    monitor: 'google_cloud_vertex_ai',
    note: 'Vertex AI: calls to generative models (Gemini and Model Garden) with their latency, time to the first token and tokens, then online prediction endpoints, their replicas and resources, Feature Store serving and Agent Engine.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Model invocations', sum('publisher.online_serving.model_invocation_count')),
          qv('Tokens', sum('publisher.online_serving.token_count')),
          qv('Avg time to first token', avg('publisher.online_serving.first_token_latencies.avg'), { precision: 0 }),
          qv('Endpoint predictions', sum('prediction.online.prediction_count')),
          qv('Prediction errors', sum('prediction.online.error_count'), { bad: true }),
        ],
        groups: [
          g('Generative model calls', [
            ts('Invocations by model', sum('publisher.online_serving.model_invocation_count', 'model_user_id'), { display: 'bars', w: 4 }),
            ts('Invocations by response code', sum('publisher.online_serving.model_invocation_count', 'response_code'), { display: 'bars', w: 4 }),
            ts('Latency and time to first token', [avg('publisher.online_serving.model_invocation_latencies.avg'), avg('publisher.online_serving.first_token_latencies.avg')], {
              alias: ['invocation', 'first token'],
              w: 4,
            }),
          ]),
          g('Tokens and throughput', [
            ts('Tokens by type', sum('publisher.online_serving.token_count', 'type'), { display: 'bars', w: 4 }),
            top('Models using the most tokens', sum('publisher.online_serving.token_count', 'model_user_id'), { w: 4 }),
            ts('Provisioned throughput consumed', sum('publisher.online_serving.consumed_token_throughput', 'model_user_id'), { w: 4 }),
          ]),
          g('Online prediction endpoints', [
            ts('Predictions by endpoint', sum('prediction.online.prediction_count', 'endpoint_id'), { display: 'bars', w: 4 }),
            ts('Responses by code', sum('prediction.online.response_count', 'response_code'), { display: 'bars', w: 4 }),
            ts('Prediction latency', avg('prediction.online.prediction_latencies.avg', 'endpoint_id'), { w: 4 }),
          ]),
          g('Endpoint replicas and resources', [
            ts('Replicas against target', [sum('prediction.online.replicas'), sum('prediction.online.target_replicas')], { alias: ['replicas', 'target'], w: 4 }),
            ts('CPU and accelerator duty cycle', [max('prediction.online.cpu.utilization'), max('prediction.online.accelerator.duty_cycle')], { alias: ['CPU', 'accelerator'], w: 4 }),
            ts('Memory by endpoint', max('prediction.online.memory.bytes_used', 'endpoint_id'), { w: 4 }),
          ]),
          g('Feature Store and Agent Engine', [
            ts('Feature online serving requests', sum('featureonlinestore.online_serving.request_count'), { display: 'bars', w: 4 }),
            ts('Feature serving latency', avg('featureonlinestore.online_serving.serving_latencies.avg'), { w: 4 }),
            ts('Agent Engine requests and latency', [sum('reasoning_engine.request_count'), avg('reasoning_engine.request_latencies.avg')], { alias: ['requests', 'latency'], w: 4 }),
          ]),
        ],
      };
    },
  },

  'serviceruntime.googleapis.com/': {
    vars: ['service', 'method'],
    labels: ['service', 'method', 'version', 'response_code', 'response_code_class', 'protocol', 'quota_metric', 'limit_name'],
    monitor: 'google_cloud_apis',
    note: 'Calls your project makes to Google APIs and to your own services behind API Gateway or Cloud Endpoints: requests by API, method and response class, latency split into backend and overhead, payload sizes, and quota usage against limits.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Requests', sum('api.request_count')),
          qv('5xx responses', sum('api.request_count', null, 'response_code_class:5xx'), { bad: true }),
          qv('4xx responses', sum('api.request_count', null, 'response_code_class:4xx')),
          qv('Avg latency', avg('api.request_latencies.avg'), { precision: 3 }),
          qv('Quota exceeded', sum('quota.ratev2.exceeded'), { bad: true }),
        ],
        groups: [
          g('Requests by API and method', [
            ts('Requests by API', sum('api.request_count', 'service'), { display: 'bars', w: 8 }),
            top('Busiest methods', sum('api.request_count', 'method'), { w: 4 }),
          ]),
          g('Errors by response class', [
            ts('Requests by response class', sum('api.request_count', 'response_code_class'), { display: 'bars', w: 4 }),
            share('Error share by API', sum('api.request_count', 'service', 'response_code_class:5xx'), sum('api.request_count', 'service'), { w: 4 }),
            top('Methods with the most errors', sum('api.request_count', 'method', '!response_code_class:2xx'), { palette: 'warm', w: 4 }),
          ]),
          g('Latency and payloads', [
            ts('Backend against overhead', [avg('api.request_latencies_backend.avg'), avg('api.request_latencies_overhead.avg')], { alias: ['backend', 'overhead'], w: 4 }),
            ts('Latency by API', avg('api.request_latencies.avg', 'service'), { w: 4 }),
            ts('Request and response size', [avg('api.request_sizes.avg'), avg('api.response_sizes.avg')], { alias: ['request', 'response'], w: 4 }),
          ]),
          g('Quota usage against limits', [
            top('Allocation quotas used', max('quota.allocation.usage', 'quota_metric'), { agg: 'last', w: 4 }),
            ts('Rate quota usage', sum('quota.rate.net_usage', 'quota_metric'), { display: 'bars', w: 4 }),
            ts('Rate quota exceeded', sum('quota.ratev2.exceeded', 'limit_name'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },

  'logging.googleapis.com/': {
    vars: ['resource_type', 'log'],
    labels: ['resource_type', 'log', 'severity', 'name'],
    monitor: 'google_cloud_logging',
    note: 'Cloud Logging for the project: log entries and bytes by resource type and severity, what ingestion is billed, sinks exporting logs (and their errors), and log-based metrics.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Log entries', sum('log_entry_count')),
          qv('Error entries', sum('log_entry_count', null, 'severity:ERROR')),
          qv('Bytes ingested', sum('billing.bytes_ingested')),
          qv('Ingested this month', max('billing.monthly_bytes_ingested'), { agg: 'last' }),
          qv('Export errors', sum('exports.error_count'), { bad: true }),
        ],
        groups: [
          g('Log volume by resource and severity', [
            ts('Entries by resource type', sum('log_entry_count', 'resource_type'), { display: 'bars', w: 6 }),
            ts('Entries by severity', sum('log_entry_count', 'severity'), { display: 'bars', w: 6 }),
            top('Noisiest logs', sum('byte_count', 'log'), { w: 6 }),
            pie('Bytes by resource type', sum('byte_count', 'resource_type'), { w: 6 }),
          ]),
          g('Ingestion billing', [
            ts('Bytes ingested by resource type', sum('billing.bytes_ingested', 'resource_type'), { display: 'bars', w: 6 }),
            ts('Ingested this month and stored', [max('billing.monthly_bytes_ingested'), max('billing.bytes_stored')], { alias: ['ingested this month', 'stored'], w: 6 }),
          ]),
          g('Sinks and log-based metrics', [
            ts('Entries exported by sink', sum('exports.log_entry_count', 'name'), { display: 'bars', w: 4 }),
            ts('Export errors by sink', sum('exports.error_count', 'name'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Log-based metric errors', sum('logs_based_metrics_error_count'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },

  'cloudtasks.googleapis.com/': {
    vars: ['location', 'queue_id'],
    labels: ['queue_id', 'target_type', 'response_code', 'api_method'],
    monitor: 'google_cloud_tasks',
    note: 'Cloud Tasks queues: tasks waiting, dispatch attempts and how their targets answered, the delay between a task’s schedule and its attempt, and API calls.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Tasks in queues', sum('queue.depth'), { agg: 'last' }),
          qv('Attempts', sum('queue.task_attempt_count')),
          qv('Failed attempts', sum('queue.task_attempt_count', null, '!response_code:OK'), { bad: true }),
          qv('Avg attempt delay', avg('queue.task_attempt_delays.avg'), { precision: 0 }),
        ],
        groups: [
          g('Queue depth and dispatch', [top('Deepest queues', max('queue.depth', 'queue_id'), { agg: 'last', w: 4 }), ts('Depth by queue', max('queue.depth', 'queue_id'), { w: 8 })]),
          g('Attempts and their outcome', [
            ts('Attempts by response code', sum('queue.task_attempt_count', 'response_code'), { display: 'bars', w: 4 }),
            share('Failure share by queue', sum('queue.task_attempt_count', 'queue_id', '!response_code:OK'), sum('queue.task_attempt_count', 'queue_id'), { w: 4 }),
            ts('Delay before an attempt', avg('queue.task_attempt_delays.avg', 'queue_id'), { w: 4 }),
          ]),
          g('Cloud Tasks API calls', [
            ts('API calls by method', sum('api.request_count', 'api_method'), { display: 'bars' }),
            ts('API calls by response code', sum('api.request_count', 'response_code'), { display: 'bars' }),
          ]),
        ],
      };
    },
  },

  'cloudkms.googleapis.com/': {
    vars: ['location'],
    labels: [],
    monitor: 'google_cloud_kms',
    note: 'Cloud KMS for the project: the peak rate of cryptographic requests, calls to an External Key Manager and their latency, and multi-region HSM and EKM quotas against their limits.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      const quota = (name, title) => ts(title, [sum(`quota.${name}.usage`), max(`quota.${name}.limit`)], { alias: ['used', 'limit'], w: 4 });
      return {
        kpis: [
          qv('Peak requests per second', max('peak_qps'), { agg: 'max' }),
          qv('EKM requests', sum('ekm.external.request_count')),
          qv('Avg EKM latency', avg('ekm.external.request_latencies.avg'), { precision: 0 }),
          qv(
            'Quota exceeded',
            [sum('quota.hsm_multiregion_symmetric_requests.exceeded'), sum('quota.hsm_multiregion_asymmetric_requests.exceeded'), sum('quota.external_kms_multiregion_requests.exceeded')],
            { formula: 'query1 + query2 + query3', bad: true },
          ),
        ],
        groups: [
          g('Request rate and External Key Manager', [
            ts('Peak requests per second by location', max('peak_qps', 'location'), { w: 4 }),
            ts('EKM requests', sum('ekm.external.request_count', 'location'), { display: 'bars', w: 4 }),
            ts('EKM latency', avg('ekm.external.request_latencies.avg', 'location'), { w: 4 }),
          ]),
          g('Multi-region quotas', [
            quota('hsm_multiregion_symmetric_requests', 'HSM symmetric requests'),
            quota('hsm_multiregion_asymmetric_requests', 'HSM asymmetric requests'),
            quota('external_kms_multiregion_requests', 'EKM requests'),
          ]),
        ],
      };
    },
  },

  'eventarc.googleapis.com/': {
    vars: ['location'],
    labels: [],
    monitor: 'google_cloud_eventarc',
    note: 'Eventarc Advanced message buses: the enrollments each bus holds against its quota, and enrollments refused once it is reached. Event delivery itself shows on the targets (Cloud Run, Workflows, Pub/Sub).',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Enrollments', sum('quota.enrollments_per_message_bus.usage'), { agg: 'last' }),
          qv('Limit per bus', max('quota.enrollments_per_message_bus.limit'), { agg: 'last' }),
          qv('Most used of the limit %', [max('quota.enrollments_per_message_bus.usage'), max('quota.enrollments_per_message_bus.limit')], { formula: '100 * query1 / query2', precision: 1 }),
          qv('Refused', sum('quota.enrollments_per_message_bus.exceeded'), { bad: true }),
        ],
        groups: [
          g('Enrollments per message bus', [
            ts('Enrollments against the limit', [max('quota.enrollments_per_message_bus.usage', 'location'), max('quota.enrollments_per_message_bus.limit', 'location')], {
              alias: ['used', 'limit'],
              w: 8,
            }),
            txt('A bus at its limit refuses new enrollments: raise the quota or spread consumers over more buses.', { w: 4 }),
          ]),
          g('Refused enrollments', [ts('Refused by location', sum('quota.enrollments_per_message_bus.exceeded', 'location'), { display: 'bars', palette: 'warm' })]),
        ],
      };
    },
  },

  'monitoring.googleapis.com/': {
    vars: ['check_id', 'checker_location'],
    labels: ['check_id', 'checker_location', 'host', 'metric_domain'],
    monitor: 'google_cloud_monitoring',
    note: 'Cloud Monitoring uptime checks seen from each checker location: whether they pass, how fast the target answers, content mismatches and days left on TLS certificates; and how many samples and time series the project sends to Cloud Monitoring.',
    build: (q) => {
      const { sum, avg, max, min } = on(q);
      return {
        kpis: [
          qv('Checks passing (lowest)', min('uptime_check.check_passed'), { agg: 'min', good: true }),
          qv('Avg check latency', avg('uptime_check.request_latency'), { precision: 0 }),
          qv('Soonest certificate expiry (days)', min('uptime_check.time_until_ssl_cert_expires'), { agg: 'min' }),
          qv('Content mismatches', max('uptime_check.content_mismatch'), { agg: 'max', bad: true }),
          qv('Samples ingested', sum('billing.samples_ingested')),
        ],
        groups: [
          g('Uptime checks by location', [
            ts('Passing by check', min('uptime_check.check_passed', 'check_id'), { w: 4 }),
            ts('Passing by checker location', min('uptime_check.check_passed', 'checker_location'), { w: 4 }),
            top('Failing checks', min('uptime_check.check_passed', 'check_id'), { order: 'asc', agg: 'min', w: 4 }),
            ts('Latency by check', avg('uptime_check.request_latency', 'check_id')),
            ts('Latency by checker location', avg('uptime_check.request_latency', 'checker_location')),
          ]),
          g('Certificates and content', [
            top('Certificates expiring soonest (days)', min('uptime_check.time_until_ssl_cert_expires', 'check_id'), { order: 'asc', agg: 'min' }),
            ts('Content mismatches by check', max('uptime_check.content_mismatch', 'check_id')),
          ]),
          g('Cloud Monitoring usage', [
            ts('Samples ingested', sum('billing.samples_ingested', 'metric_domain'), { display: 'bars', w: 4 }),
            ts('Bytes ingested', sum('billing.bytes_ingested', 'metric_domain'), { display: 'bars', w: 4 }),
            ts('Active time series', max('stats.num_time_series'), { w: 4 }),
          ]),
        ],
      };
    },
  },
};
