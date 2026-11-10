// Dashboards designed per Azure resource type, as Datadog has one for each service of its Azure integration: each follows
// its service's model (what a vault, a cache or a namespace is judged by) instead of one template. The structure (an
// overview with the service's logo, its resources by Resource Health status, key numbers and monitors, then a group per
// question, its logs last) follows the kind of dashboards Datadog ships; titles, notes and layouts are ours, and only
// names our Azure crawler publishes are charted (azure/naming.rs: a metric's bare name carries its primary aggregation,
// `.max` its maximum, `azure.<type>.count` the resources with their `status`; the dimensions in SPLITS become lowercase
// tags). scripts/cloud-service-dashboards.mjs turns them into dashboards; the types left out get its generated layout.
import { chg, group, heat, qv, tbl, top, tree, ts, txt } from './aws-service-layouts.mjs';
import { metric } from './cloud-dashboards.mjs';

// ---------------------------------------------------------------- widgets

/** A themed group, in Azure's blue. */
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

/** The queries of a type's metrics, by their name after the type's prefix (`percentage_cpu`). */
const on = (q) => {
  const named =
    (space) =>
    (name, ...rest) =>
      q[space](`${q.prefix}.${name}`, ...rest);
  return { sum: named('sum'), avg: named('avg'), max: named('max'), min: named('min'), count: q.count };
};

/** The type's resources now, by Resource Health status (azure.<type>.count, one series per resource). */
const running = (count, label) => qv(label, count(null, 'status:running'), { agg: 'last', good: true });
const notRunning = (count) => qv('Degraded or unavailable', count(null, '!status:running'), { agg: 'last', bad: true });
const health = (count, label) => [running(count, label), notRunning(count)];
const byStatus = (count, title = 'Resources by health status', o = {}) => ts(title, count('status'), { display: 'bars', palette: 'cool', ...o });

/** A per-resource rate: `100 * a / b` by resource. */
const rate = (title, a, b, o = {}) => ts(title, [a, b], { formulas: [['100 * query1 / query2', '%']], palette: 'warm', ...o });
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

// ---------------------------------------------------------------- the resource types

/**
 * resource type → { vars: the template variables after subscription, resource group, region and name (the dimension
 * tags the crawler splits by), monitor: the `integration:` tag of its monitors, note: what it shows (our words),
 * logs: [[title, query]], build({ sum, avg, max, min, count, prefix, f }) → { kpis, groups } }.
 */
export const AZURE_LAYOUTS = {
  'Microsoft.Compute/virtualMachines': {
    vars: [],
    monitor: 'azure_vm',
    note: 'Virtual machines seen from the host: Resource Health, CPU and burst credits, memory, the OS and data disks, and network flows. Processes and file systems inside the VM come from the Agent.',
    logs: [['Compute logs', 'source:azure.compute']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'VMs running'),
          qv('Avg CPU', avg('percentage_cpu'), { precision: 1 }),
          qv('Busiest VM CPU', max('percentage_cpu'), { precision: 1 }),
          qv('Avg available memory', avg('available_memory_percentage'), { precision: 1 }),
          qv('Network in', sum('network_in_total')),
        ],
        groups: [
          g('Availability', [
            byStatus(count, 'VMs by health status', { w: 4 }),
            ts('VM availability', min('vm_availability_metric', 'name'), { w: 4 }),
            top('Least available VMs', min('vm_availability_metric', 'name'), { order: 'asc', w: 4 }),
          ]),
          g('Compute', [
            top('Busiest VMs', avg('percentage_cpu', 'name'), { w: 4, h: 4, palette: 'orange' }),
            ts('CPU by VM', avg('percentage_cpu', 'name'), { w: 8 }),
            heat('CPU spread across VMs', avg('percentage_cpu', 'name'), { w: 4 }),
            ts('CPU credits left (B-series)', avg('cpu_credits_remaining', 'name'), { w: 4 }),
          ]),
          g('Memory', [
            ts('Available memory by VM', avg('available_memory_bytes', 'name'), { w: 8 }),
            top('Least memory available', min('available_memory_percentage', 'name'), { order: 'asc', w: 4 }),
          ]),
          g('OS and data disks', [
            ts('Disk throughput', [sum('disk_read_bytes'), sum('disk_write_bytes')], { alias: ['read', 'written'], w: 4 }),
            ts('OS disk latency', avg('os_disk_latency', 'name'), { w: 4 }),
            ts('Data disk latency', avg('data_disk_latency', 'name'), { w: 4 }),
            top('Deepest OS disk queues', max('os_disk_queue_depth', 'name'), { w: 4 }),
            ts('OS disk IOPS used (% of the cap)', max('os_disk_iops_consumed_percentage', 'name'), { w: 4 }),
            ts('Uncached bandwidth used (% of the VM cap)', max('vm_uncached_bandwidth_consumed_percentage', 'name'), { w: 4 }),
          ]),
          g('Network', [
            mirror('Traffic in and out', sum('network_in_total'), sum('network_out_total')),
            top('Most traffic sent', sum('network_out_total', 'name')),
            ts('Flows', [avg('inbound_flows'), avg('outbound_flows')], { alias: ['inbound', 'outbound'], w: 12 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Compute/virtualMachineScaleSets': {
    vars: [],
    monitor: 'azure_vm_scale_set',
    note: 'Scale sets as a whole: their health, how busy their instances are on average and at peak, memory, disks and network flows.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Scale sets running'),
          qv('Avg CPU', avg('percentage_cpu'), { precision: 1 }),
          qv('Busiest scale set CPU', max('percentage_cpu'), { precision: 1 }),
          qv('Avg available memory', avg('available_memory_percentage'), { precision: 1 }),
          qv('Network out', sum('network_out_total')),
        ],
        groups: [
          g('Scale sets', [
            pie('By health status', q.count('status'), { agg: 'last' }),
            tbl(
              'Scale sets',
              [
                ['CPU %', avg('percentage_cpu', 'name')],
                ['Available memory %', avg('available_memory_percentage', 'name')],
                ['Network out', sum('network_out_total', 'name'), 'sum'],
              ],
              { w: 8, h: 2 },
            ),
          ]),
          g('CPU and memory', [
            ts('CPU by scale set', avg('percentage_cpu', 'name')),
            ts('Available memory by scale set', avg('available_memory_bytes', 'name')),
            ts('CPU credits consumed', sum('cpu_credits_consumed', 'name'), { display: 'bars' }),
            top('Least memory available', min('available_memory_percentage', 'name'), { order: 'asc' }),
          ]),
          g('Instance disks', [
            ts('Disk bytes read and written', [sum('disk_read_bytes'), sum('disk_write_bytes')], { alias: ['read', 'written'], w: 4 }),
            ts('OS disk latency', avg('os_disk_latency', 'name'), { w: 4 }),
            ts('OS disk queue depth', max('os_disk_queue_depth', 'name'), { w: 4 }),
          ]),
          g('Network and flows', [
            mirror('Traffic in and out', sum('network_in_total'), sum('network_out_total')),
            ts('Flows created per second (peak)', [max('inbound_flows_maximum_creation_rate'), max('outbound_flows_maximum_creation_rate')], { alias: ['inbound', 'outbound'] }),
          ]),
        ],
      };
    },
  },

  'Microsoft.ContainerService/managedClusters': {
    vars: ['nodepool', 'node'],
    monitor: 'azure_kubernetes_service',
    note: 'AKS clusters from Azure Monitor’s platform metrics: node CPU, memory and disks per node pool, pods by phase, the cluster autoscaler and the managed control plane. Workloads in detail come from the Agent (Kubernetes Explorer).',
    logs: [['AKS control plane logs', 'source:azure.containerservice']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Clusters running'),
          qv('Busiest node CPU', max('node_cpu_usage_percentage'), { precision: 1 }),
          qv('Busiest node memory', max('node_memory_working_set_percentage'), { precision: 1 }),
          qv('Unschedulable pods', sum('cluster_autoscaler_unschedulable_pods_count'), { agg: 'last', bad: true }),
          qv('API server CPU', max('apiserver_cpu_usage_percentage'), { precision: 1 }),
        ],
        groups: [
          g('Nodes', [
            top('Busiest nodes', max('node_cpu_usage_percentage', 'node'), { w: 4, h: 4, palette: 'orange' }),
            ts('CPU by node pool', max('node_cpu_usage_percentage', 'nodepool'), { w: 8 }),
            ts('Memory working set by node pool', max('node_memory_working_set_percentage', 'nodepool'), { w: 4 }),
            top('Fullest node disks', max('node_disk_usage_percentage', 'node'), { w: 4 }),
          ]),
          g('Pods and scheduling', [
            ts('Pods by phase', sum('kube_pod_status_phase', 'phase'), { display: 'bars', w: 6 }),
            ts('Unschedulable pods', sum('cluster_autoscaler_unschedulable_pods_count', 'name'), { display: 'bars', palette: 'warm', w: 6 }),
            ts('Scheduling attempts', avg('scheduler_schedule_attempts_rate', 'name'), { w: 6 }),
            ts('Ready pods', sum('kube_pod_status_ready', 'name'), { w: 6 }),
          ]),
          g('Control plane', [
            ts('API server CPU and memory', [max('apiserver_cpu_usage_percentage'), max('apiserver_memory_usage_percentage')], { alias: ['CPU %', 'memory %'], w: 4 }),
            ts('Inflight API requests', sum('apiserver_current_inflight_requests', 'name'), { w: 4 }),
            ts('etcd database used', max('etcd_database_usage_percentage', 'name'), { w: 4 }),
          ]),
          g('Cluster autoscaler', [
            ts('Unneeded nodes', sum('cluster_autoscaler_unneeded_nodes_count', 'name'), { display: 'bars', w: 4 }),
            ts('Safe to autoscale', min('cluster_autoscaler_cluster_safe_to_autoscale', 'name'), { w: 4 }),
            ts('Scale-down cooldown', max('cluster_autoscaler_scale_down_in_cooldown', 'name'), { w: 4 }),
          ]),
          g('Allocatable capacity and network', [
            ts('Allocatable CPU cores', sum('kube_node_status_allocatable_cpu_cores', 'name'), { w: 4 }),
            ts('Allocatable memory', sum('kube_node_status_allocatable_memory_bytes', 'name'), { w: 4 }),
            mirror('Node network', max('node_network_in_bytes'), max('node_network_out_bytes'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Web/sites': {
    vars: ['instance'],
    monitor: 'azure_app_services',
    note: 'Web apps and function apps: requests and HTTP status codes, response time, each instance’s CPU time, memory and queue, and function executions. Function apps share this resource type.',
    logs: [['App Service logs', 'source:azure.web']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Apps running'),
          qv('Requests', sum('requests')),
          qv('Server errors (5xx)', sum('http5xx'), { bad: true }),
          qv('Avg response time', avg('http_response_time'), { precision: 2 }),
          qv('Function executions', sum('function_execution_count')),
        ],
        groups: [
          g('Traffic', [
            ts('Requests by app', sum('requests', 'name'), { display: 'bars', w: 8 }),
            top('Busiest apps', sum('requests', 'name'), { w: 4 }),
            ts('Requests by instance', sum('requests', 'instance'), { w: 6 }),
            mirror('Data in and out', sum('bytes_received'), sum('bytes_sent'), { w: 6 }),
          ]),
          g('HTTP status codes', [
            ts('Responses by class', [sum('http2xx'), sum('http3xx'), sum('http4xx'), sum('http5xx')], { alias: ['2xx', '3xx', '4xx', '5xx'], display: 'bars', w: 6 }),
            rate('5xx rate by app', sum('http5xx', 'name'), sum('requests', 'name'), { w: 6 }),
            ts('Client errors', [sum('http401'), sum('http403'), sum('http404')], { alias: ['401', '403', '404'], w: 6 }),
            top('Apps with the most 5xx', sum('http5xx', 'name'), { palette: 'warm', w: 6 }),
          ]),
          g('Response time', [
            ts('Response time by app', avg('http_response_time', 'name'), { w: 8 }),
            top('Slowest apps', avg('http_response_time', 'name'), { w: 4 }),
            heat('Response time across instances', avg('http_response_time', 'instance'), { w: 12 }),
          ]),
          g('Instances', [
            ts('CPU time by instance', sum('cpu_time', 'instance'), { w: 4 }),
            ts('Memory working set by instance', avg('memory_working_set', 'instance'), { w: 4 }),
            ts('Requests in the application queue', max('requests_in_application_queue', 'instance'), { palette: 'warm', w: 4 }),
            ts('Connections', max('app_connections', 'name'), { w: 4 }),
            ts('Threads', avg('threads', 'name'), { w: 4 }),
            ts('Health check status', avg('health_check_status', 'name'), { w: 4 }),
          ]),
          g('Functions', [ts('Executions by function app', sum('function_execution_count', 'name'), { display: 'bars' }), ts('Execution units (MB-ms)', sum('function_execution_units', 'name'))]),
        ],
      };
    },
  },

  'Microsoft.Web/serverFarms': {
    vars: ['instance'],
    monitor: 'azure_app_service_plan',
    note: 'App Service plans: the workers every app on the plan shares, their CPU and memory, HTTP and disk queues, sockets and data in and out. Scale out when the queues grow.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Plans running'),
          qv('Avg CPU', avg('cpu_percentage'), { precision: 1 }),
          qv('Avg memory', avg('memory_percentage'), { precision: 1 }),
          qv('Longest HTTP queue', max('http_queue_length'), { bad: true }),
          qv('Longest disk queue', max('disk_queue_length')),
        ],
        groups: [
          g('Plan load', [
            ts('CPU by plan', avg('cpu_percentage', 'name'), { w: 4 }),
            ts('Memory by plan', avg('memory_percentage', 'name'), { w: 4 }),
            top('Busiest plans', avg('cpu_percentage', 'name'), { w: 4 }),
          ]),
          g('Workers', [heat('CPU across workers', avg('cpu_percentage', 'instance')), ts('Memory by worker', avg('memory_percentage', 'instance'))]),
          g('Queues', [ts('HTTP queue length', max('http_queue_length', 'name'), { palette: 'warm' }), ts('Disk queue length', max('disk_queue_length', 'name'), { palette: 'warm' })]),
          g('Sockets and TCP', [
            ts('Sockets', [avg('socket_inbound_all'), avg('socket_outbound_all'), avg('socket_loopback')], { alias: ['inbound', 'outbound', 'loopback'], w: 4 }),
            ts('TCP states', [avg('tcp_established'), avg('tcp_time_wait'), avg('tcp_close_wait')], { alias: ['established', 'time wait', 'close wait'], w: 4 }),
            mirror('Data in and out', sum('bytes_received'), sum('bytes_sent'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Sql/servers/databases': {
    vars: [],
    monitor: 'azure_sql_database',
    note: 'Single databases: CPU, DTU or vCore use, workers and sessions, connections that succeed and fail, deadlocks, storage and log I/O.',
    logs: [['SQL Database logs', 'source:azure.sql']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Databases online'),
          qv('Avg CPU', avg('cpu_percent'), { precision: 1 }),
          qv('Avg DTU', avg('dtu_consumption_percent'), { precision: 1 }),
          qv('Failed connections', sum('connection_failed'), { bad: true }),
          qv('Deadlocks', sum('deadlock'), { bad: true }),
        ],
        groups: [
          g('Resource use', [
            top('Busiest databases', avg('cpu_percent', 'name'), { w: 4, h: 4, palette: 'orange' }),
            ts('CPU by database', avg('cpu_percent', 'name'), { w: 8 }),
            ts('DTU by database', avg('dtu_consumption_percent', 'name'), { w: 8 }),
            ts('Workers and sessions', [max('workers_percent'), max('sessions_percent')], { alias: ['workers %', 'sessions %'], w: 6 }),
            ts('Data and log I/O', [avg('physical_data_read_percent'), avg('log_write_percent')], { alias: ['data I/O %', 'log I/O %'], w: 6 }),
          ]),
          g('Connections', [
            ts('Connections', [sum('connection_successful'), sum('connection_failed'), sum('connection_failed_user_error')], {
              alias: ['successful', 'failed (system)', 'failed (user)'],
              display: 'bars',
            }),
            top('Most failed connections', sum('connection_failed', 'name'), { palette: 'warm' }),
            ts('Blocked by the firewall', sum('blocked_by_firewall', 'name'), { display: 'bars', palette: 'warm', w: 12 }),
          ]),
          g('Storage', [
            top('Fullest databases', max('storage_percent', 'name'), { w: 4 }),
            ts('Data space used', avg('storage', 'name'), { w: 4 }),
            ts('Data space allocated', avg('allocated_data_storage', 'name'), { w: 4 }),
          ]),
          g('Locks and availability', [
            ts('Deadlocks by database', sum('deadlock', 'name'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Availability', min('availability', 'name'), { w: 4 }),
            ts('Buffer cache hit ratio', avg('cache_hit_percent', 'name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Sql/managedInstances': {
    vars: [],
    monitor: 'azure_sql_managed_instance',
    note: 'Managed instances: vCores and CPU, workers, the storage used against what is reserved, I/O requests and bytes, and the log rate against its limit.',
    logs: [['Managed instance logs', 'source:azure.sql']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Instances running'),
          qv('vCores', sum('virtual_core_count'), { agg: 'last' }),
          qv('Avg CPU', avg('avg_cpu_percent'), { precision: 1 }),
          qv('Storage used (MB)', sum('storage_space_used_mb'), { agg: 'last' }),
          qv('Storage reserved (MB)', sum('reserved_storage_mb'), { agg: 'last' }),
        ],
        groups: [
          g('CPU and workers', [
            ts('CPU by instance', avg('avg_cpu_percent', 'name'), { w: 4 }),
            ts('Workers', max('avg_workers_percent', 'name'), { w: 4 }),
            top('Busiest instances', avg('avg_cpu_percent', 'name'), { w: 4 }),
          ]),
          g('Storage against its reservation', [
            ts('Storage used (% of reserved)', [avg('storage_space_used_mb', 'name'), avg('reserved_storage_mb', 'name')], { formulas: [['100 * query1 / query2', 'used %']], w: 8 }),
            tbl(
              'Instances',
              [
                ['Used (MB)', avg('storage_space_used_mb', 'name')],
                ['Reserved (MB)', avg('reserved_storage_mb', 'name')],
                ['vCores', avg('virtual_core_count', 'name')],
              ],
              { w: 4, h: 2 },
            ),
          ]),
          g('I/O', [
            ts('I/O requests', sum('io_requests', 'name'), { w: 4 }),
            ts('Bytes read and written', [sum('io_bytes_read'), sum('io_bytes_written')], { alias: ['read', 'written'], w: 4 }),
            ts('Log rate against its limit (MB/s)', [avg('log_rate_mbps'), avg('log_rate_limit_mbps')], { alias: ['log rate', 'limit'], w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.DBforPostgreSQL/flexibleServers': {
    vars: [],
    monitor: 'azure_db_for_postgresql_flexible_server',
    note: 'PostgreSQL flexible servers: CPU, memory and IOPS, connections against the limit, storage and transaction logs, replica lag, commits and rollbacks.',
    logs: [['PostgreSQL logs', 'source:azure.dbforpostgresql']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Servers running'),
          qv('Avg CPU', avg('cpu_percent'), { precision: 1 }),
          qv('Avg memory', avg('memory_percent'), { precision: 1 }),
          qv('Fullest storage', max('storage_percent'), { precision: 1 }),
          qv('Active connections', sum('active_connections'), { agg: 'avg' }),
        ],
        groups: [
          g('Server load', [ts('CPU by server', avg('cpu_percent', 'name'), { w: 4 }), ts('Memory by server', avg('memory_percent', 'name'), { w: 4 }), ts('IOPS', avg('iops', 'name'), { w: 4 })]),
          g('Connections', [
            ts('Active connections', avg('active_connections', 'name'), { w: 4 }),
            ts('Connections used (% of max)', [avg('active_connections', 'name'), max('max_connections', 'name')], { formulas: [['100 * query1 / query2', 'used %']], palette: 'warm', w: 4 }),
            ts('Succeeded and failed', [sum('connections_succeeded'), sum('connections_failed')], { alias: ['succeeded', 'failed'], display: 'bars', w: 4 }),
          ]),
          g('Storage and logs', [
            top('Fullest servers', max('storage_percent', 'name'), { w: 4 }),
            ts('Storage used', avg('storage_used', 'name'), { w: 4 }),
            ts('Transaction logs and backups', [avg('txlogs_storage_used'), avg('backup_storage_used')], { alias: ['transaction logs', 'backups'], w: 4 }),
          ]),
          g('Transactions and replication', [
            ts('Commits and rollbacks', [sum('xact_commit'), sum('xact_rollback')], { alias: ['commits', 'rollbacks'], w: 4 }),
            ts('Read replica lag', max('physical_replication_delay_in_seconds', 'name'), { palette: 'warm', w: 4 }),
            ts('Transaction IDs used (wraparound)', max('maximum_used_transaction_ids', 'name'), { w: 4 }),
            ts('Deadlocks', sum('deadlocks', 'name'), { display: 'bars', palette: 'warm', w: 6 }),
            mirror('Network in and out', sum('network_bytes_ingress'), sum('network_bytes_egress'), { w: 6 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.DBforMySQL/flexibleServers': {
    vars: [],
    monitor: 'azure_db_for_mysql_flexible_server',
    note: 'MySQL flexible servers: host CPU, memory and storage I/O, connections, queries and slow queries, InnoDB’s buffer pool and locks, storage and replication lag.',
    logs: [['MySQL logs', 'source:azure.dbformysql']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Servers running'),
          qv('Avg CPU', avg('cpu_percent'), { precision: 1 }),
          qv('Avg memory', avg('memory_percent'), { precision: 1 }),
          qv('Queries', sum('queries')),
          qv('Slow queries', sum('slow_queries'), { bad: true }),
        ],
        groups: [
          g('Host', [
            ts('CPU by server', avg('cpu_percent', 'name'), { w: 4 }),
            ts('Memory by server', avg('memory_percent', 'name'), { w: 4 }),
            ts('Storage I/O', avg('io_consumption_percent', 'name'), { w: 4 }),
          ]),
          g('Queries and statements', [
            ts('Queries by server', sum('queries', 'name'), { display: 'bars', w: 4 }),
            ts('Statements', [sum('com_select'), sum('com_insert'), sum('com_update'), sum('com_delete')], { alias: ['select', 'insert', 'update', 'delete'], w: 4 }),
            top('Most slow queries', sum('slow_queries', 'name'), { palette: 'warm', w: 4 }),
          ]),
          g('Connections and threads', [
            ts('Active connections', avg('active_connections', 'name'), { w: 4 }),
            ts('Aborted connections', sum('aborted_connections', 'name'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Threads running', sum('threads_running', 'name'), { w: 4 }),
          ]),
          g('InnoDB', [
            ts('Buffer pool reads from disk (% of requests)', [sum('innodb_buffer_pool_reads', 'name'), sum('innodb_buffer_pool_read_requests', 'name')], {
              formulas: [['100 * query1 / query2', 'miss %']],
              palette: 'warm',
              w: 4,
            }),
            ts('Row lock waits and deadlocks', [sum('innodb_row_lock_waits'), sum('lock_deadlocks')], { alias: ['row lock waits', 'deadlocks'], w: 4 }),
            ts('History list length', sum('trx_rseg_history_len', 'name'), { w: 4 }),
          ]),
          g('Storage and replicas', [
            top('Fullest servers', max('storage_percent', 'name'), { w: 4 }),
            ts('Replication lag', max('replication_lag', 'name'), { palette: 'warm', w: 4 }),
            ts('Data and binlog storage', [avg('data_storage_used'), avg('binlog_storage_used')], { alias: ['data', 'binlogs'], w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.DocumentDB/databaseAccounts': {
    vars: ['databasename', 'collectionname'],
    monitor: 'azure_cosmosdb',
    note: 'Cosmos DB accounts: requests by status code, request units consumed against what is provisioned, throttled (429) requests, server-side latency, storage and documents per container.',
    logs: [['Cosmos DB logs', 'source:azure.documentdb']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Accounts running'),
          qv('Requests', sum('total_requests')),
          qv('Throttled (429)', sum('total_requests', null, 'statuscode:429'), { bad: true }),
          qv('Request units', sum('total_request_units')),
          qv('Hottest partition', max('normalized_ruconsumption'), { precision: 1 }),
        ],
        groups: [
          g('Requests', [
            ts('Requests by status code', sum('total_requests', 'statuscode'), { display: 'bars', w: 8 }),
            pie('Share by container', sum('total_requests', 'collectionname')),
            ts('Requests by database', sum('total_requests', 'databasename'), { w: 6 }),
            top('Busiest containers', sum('total_requests', 'collectionname'), { w: 6 }),
          ]),
          g('Throttling', [
            ts('Throttled requests by container', sum('total_requests', 'collectionname', 'statuscode:429'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Normalized RU consumption', max('normalized_ruconsumption', 'collectionname'), { palette: 'warm', w: 4 }),
            top('Most throttled containers', sum('total_requests', 'collectionname', 'statuscode:429'), { palette: 'warm', w: 4 }),
          ]),
          g('Request units', [
            ts('RU consumed by container', sum('total_request_units', 'collectionname'), { w: 4 }),
            ts('Provisioned throughput', max('provisioned_throughput', 'collectionname'), { w: 4 }),
            ts('Autoscale maximum', max('autoscale_max_throughput', 'collectionname'), { w: 4 }),
          ]),
          g('Server-side latency', [
            ts('Direct mode', avg('server_side_latency_direct', 'collectionname'), { w: 4 }),
            ts('Gateway mode', avg('server_side_latency_gateway', 'collectionname'), { w: 4 }),
            ts('Service availability', min('service_availability', 'name'), { w: 4 }),
          ]),
          g('Data per container', [
            tree('Data stored', sum('data_usage_v2', 'collectionname'), { agg: 'last' }),
            ts('Index size', sum('index_usage_v2', 'collectionname')),
            ts('Documents', sum('document_count_v2', 'collectionname'), { w: 12 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Storage/storageAccounts': {
    vars: ['apiname', 'responsetype'],
    monitor: 'azure_storage',
    note: 'Storage accounts: availability, transactions by API and response type, end-to-end latency against server latency (the difference is the client and the network), ingress and egress, and hourly capacity.',
    logs: [['Storage logs', 'source:azure.storage']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Accounts running'),
          qv('Availability', avg('availability'), { precision: 2 }),
          qv('Transactions', sum('transactions')),
          qv('Failed transactions', sum('transactions', null, '!responsetype:success'), { bad: true }),
          qv('Avg end-to-end latency', avg('success_e2elatency'), { precision: 1 }),
        ],
        groups: [
          g('Availability and errors', [
            ts('Availability by account', avg('availability', 'name'), { w: 4 }),
            ts('Transactions by response type', sum('transactions', 'responsetype'), { display: 'bars', w: 4 }),
            top('Most failed transactions', sum('transactions', 'name', '!responsetype:success'), { palette: 'warm', w: 4 }),
          ]),
          g('Transactions', [
            ts('Transactions by API', sum('transactions', 'apiname'), { display: 'bars', w: 8 }),
            top('Most called APIs', sum('transactions', 'apiname'), { w: 4 }),
            ts('Transactions by account', sum('transactions', 'name'), { w: 12 }),
          ]),
          g('Latency', [
            ts('End-to-end against server', [avg('success_e2elatency'), avg('success_server_latency')], { alias: ['end-to-end', 'server'], w: 6 }),
            ts('End-to-end latency by API', avg('success_e2elatency', 'apiname'), { w: 6 }),
            top('Slowest accounts', avg('success_e2elatency', 'name'), { w: 6 }),
            ts('Peak server latency', max('success_server_latency.max', 'name'), { w: 6 }),
          ]),
          g('Ingress and egress', [mirror('Bytes in and out', sum('ingress'), sum('egress'), { w: 8 }), top('Most egress', sum('egress', 'name'), { w: 4 })]),
          g('Capacity (hourly)', [
            ts('Used capacity', avg('used_capacity', 'name'), { display: 'area', span: '1w', w: 4 }),
            top('Largest accounts', avg('used_capacity', 'name'), { agg: 'last', span: '1w', w: 4 }),
            chg('Against the week before', avg('used_capacity', 'name'), { increaseGood: false, w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.KeyVault/vaults': {
    vars: ['activitytype'],
    monitor: 'azure_key_vault',
    note: 'Key vaults: API hits by activity (secrets, keys, certificates, authentication), their results by status code, availability, latency and how close each vault is to its throttling limits (saturation).',
    logs: [['Key Vault audit logs', 'source:azure.keyvault']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Vaults running'),
          qv('API hits', sum('service_api_hit')),
          qv('Availability', avg('availability'), { precision: 2 }),
          qv('Avg latency', avg('service_api_latency'), { precision: 1 }),
          qv('Peak saturation', max('saturation_shoebox'), { precision: 1 }),
        ],
        groups: [
          g('API activity', [
            ts('Hits by activity type', sum('service_api_hit', 'activitytype'), { display: 'bars', w: 6 }),
            top('Most used operations', sum('service_api_hit', 'activityname'), { w: 6 }),
            ts('Hits by vault', sum('service_api_hit', 'name'), { w: 12 }),
          ]),
          g('Results', [
            ts('Results by status code', sum('service_api_result', 'statuscode'), { display: 'bars', w: 6 }),
            top('Vaults with the most failed calls', sum('service_api_result', 'name', '!statuscode:200'), { palette: 'warm', w: 6 }),
          ]),
          g('Availability and latency', [
            ts('Availability by vault', min('availability', 'name'), { w: 4 }),
            ts('Latency by activity type', avg('service_api_latency', 'activitytype'), { w: 4 }),
            top('Slowest operations', avg('service_api_latency', 'activityname'), { w: 4 }),
          ]),
          g('Saturation', [
            ts('Saturation by vault', max('saturation_shoebox', 'name'), { palette: 'warm', w: 8 }),
            txt('Saturation is the share of the vault’s request limits in use; Key Vault throttles at 100%.', { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Cache/redis': {
    vars: ['shardid'],
    monitor: 'azure_redis_cache',
    note: 'Azure Cache for Redis: hits against misses, server load and CPU per shard, memory used, connected clients, evictions and commands.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Caches running'),
          qv('Cache hits', sum('cachehits')),
          qv('Cache misses', sum('cachemisses')),
          qv('Connected clients', max('connectedclients'), { agg: 'max' }),
          qv('Peak server load', max('server_load'), { precision: 1 }),
        ],
        groups: [
          g('Hits and misses', [
            ts('Hits and misses', [sum('cachehits'), sum('cachemisses')], { alias: ['hits', 'misses'], display: 'bars', w: 4 }),
            ts('Hit rate by cache', [sum('cachehits', 'name'), sum('cachemisses', 'name')], { formulas: [['100 * query1 / (query1 + query2)', 'hit %']], w: 4 }),
            ts('Evicted and expired keys', [sum('evictedkeys'), sum('expiredkeys')], { alias: ['evicted', 'expired'], palette: 'warm', w: 4 }),
          ]),
          g('Server load by shard', [
            ts('Server load', max('server_load', 'shardid'), { palette: 'warm', w: 4 }),
            ts('CPU', max('percent_processor_time', 'shardid'), { w: 4 }),
            ts('Operations per second', max('operations_per_second', 'name'), { w: 4 }),
          ]),
          g('Memory', [
            ts('Memory used (%)', max('usedmemorypercentage', 'name'), { w: 4 }),
            ts('Memory used by shard', max('usedmemory', 'shardid'), { w: 4 }),
            top('Fullest caches', max('usedmemorypercentage', 'name'), { w: 4 }),
          ]),
          g('Clients and commands', [
            ts('Connected clients', max('connectedclients', 'name'), { w: 4 }),
            ts('Gets and sets', [sum('getcommands'), sum('setcommands')], { alias: ['gets', 'sets'], w: 4 }),
            ts('Errors', max('errors', 'name'), { palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Cache/redisEnterprise': {
    vars: [],
    monitor: 'azure_managed_redis',
    note: 'Azure Managed Redis and Cache for Redis Enterprise clusters: operations, hits and evictions, memory and keys, server load and latency, geo-replication health.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Clusters running'),
          qv('Peak operations/s', max('operations_per_second'), { agg: 'max' }),
          qv('Peak server load', max('server_load'), { precision: 1 }),
          qv('Memory used', max('usedmemorypercentage'), { precision: 1 }),
          qv('Connected clients', max('connectedclients'), { agg: 'max' }),
        ],
        groups: [
          g('Throughput', [
            ts('Operations per second', max('operations_per_second', 'name'), { w: 4 }),
            ts('Gets and sets', [avg('getcommands'), avg('setcommands')], { alias: ['gets', 'sets'], w: 4 }),
            ts('Read and write bandwidth', [max('cache_read'), max('cache_write')], { alias: ['read', 'write'], w: 4 }),
          ]),
          g('Keyspace', [
            ts('Hits and misses', [avg('cachehits'), avg('cachemisses')], { alias: ['hits', 'misses'], w: 4 }),
            ts('Evicted and expired keys', [avg('evictedkeys'), avg('expiredkeys')], { alias: ['evicted', 'expired'], palette: 'warm', w: 4 }),
            ts('Keys', max('totalkeys', 'name'), { w: 4 }),
          ]),
          g('Server and replication', [
            ts('Server load', max('server_load', 'name'), { palette: 'warm', w: 4 }),
            ts('Latency (µs)', avg('cache_latency', 'name'), { w: 4 }),
            ts('Geo-replication healthy', min('geo_replication_healthy', 'name'), { w: 4 }),
            ts('Memory used', max('usedmemory', 'name'), { w: 12 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Network/loadBalancers': {
    vars: ['frontendipaddress', 'backendipaddress'],
    monitor: 'azure_load_balancer',
    note: 'Standard load balancers: data path availability per frontend, health probe status per backend, bytes, packets and SYNs, and the SNAT ports backends use for outbound connections.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Load balancers running'),
          qv('Data path availability', avg('vip_availability'), { precision: 2 }),
          qv('Health probe status', avg('dip_availability'), { precision: 2 }),
          qv('Bytes', sum('byte_count')),
          qv('SNAT ports used', sum('used_snat_ports'), { agg: 'avg' }),
        ],
        groups: [
          g('Frontends and backends', [
            ts('Data path availability by frontend', avg('vip_availability', 'frontendipaddress'), { w: 4 }),
            ts('Health probes by backend', avg('dip_availability', 'backendipaddress'), { w: 4 }),
            top('Least healthy backends', min('dip_availability', 'backendipaddress'), { order: 'asc', w: 4 }),
          ]),
          g('Load balanced traffic', [
            ts('Bytes by frontend', sum('byte_count', 'frontendipaddress'), { display: 'bars', w: 4 }),
            ts('Packets', sum('packet_count', 'name'), { w: 4 }),
            ts('SYN packets', sum('syncount', 'name'), { w: 4 }),
          ]),
          g('Outbound SNAT', [
            ts('SNAT ports used (% of allocated)', [sum('used_snat_ports', 'backendipaddress'), sum('allocated_snat_ports', 'backendipaddress')], {
              formulas: [['100 * query1 / query2', 'used %']],
              palette: 'warm',
              w: 6,
            }),
            ts('SNAT connections', sum('snat_connection_count', 'name'), { w: 6 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Network/applicationGateways': {
    vars: ['listener', 'backendsettingspool'],
    monitor: 'azure_application_gateway',
    note: 'Application gateways: requests and response status, healthy and unhealthy backend hosts per pool, the time spent in the gateway and at the backends, capacity units and the Web Application Firewall’s matches.',
    logs: [['Application Gateway logs', 'source:azure.network']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Gateways running'),
          qv('Requests', sum('total_requests')),
          qv('Failed requests', sum('failed_requests'), { bad: true }),
          qv('Unhealthy hosts', max('unhealthy_host_count'), { agg: 'max', bad: true }),
          qv('Avg total time', avg('application_gateway_total_time'), { precision: 1 }),
        ],
        groups: [
          g('Requests and responses', [
            ts('Requests by backend pool', sum('total_requests', 'backendsettingspool'), { display: 'bars', w: 4 }),
            ts('Responses by status group', sum('response_status', 'httpstatusgroup'), { display: 'bars', w: 4 }),
            ts('Failed requests by backend pool', sum('failed_requests', 'backendsettingspool'), { palette: 'warm', w: 4 }),
          ]),
          g('Backend health', [
            ts('Healthy hosts', avg('healthy_host_count', 'backendsettingspool'), { w: 4 }),
            ts('Unhealthy hosts', max('unhealthy_host_count', 'backendsettingspool'), { palette: 'warm', w: 4 }),
            ts('Requests per healthy host', avg('avg_request_count_per_healthy_host', 'backendsettingspool'), { w: 4 }),
          ]),
          g('Where the time goes', [
            ts('Gateway total time by listener', avg('application_gateway_total_time', 'listener'), { w: 6 }),
            ts('Backend connect, first byte and last byte', [avg('backend_connect_time'), avg('backend_first_byte_response_time'), avg('backend_last_byte_response_time')], {
              alias: ['connect', 'first byte', 'last byte'],
              w: 6,
            }),
            ts('Client round trip', avg('client_rtt', 'listener'), { w: 12 }),
          ]),
          g('Capacity', [
            ts('Capacity and compute units', [avg('capacity_units'), avg('compute_units')], { alias: ['capacity units', 'compute units'], w: 4 }),
            ts('Current connections', sum('current_connections', 'name'), { w: 4 }),
            ts('Throughput', avg('throughput', 'name'), { w: 4 }),
          ]),
          g('Web Application Firewall', [
            ts('WAF requests', sum('azwaf_total_requests', 'name'), { w: 4 }),
            ts('Matched and blocked', [sum('matched_count'), sum('blocked_count')], { alias: ['matched', 'blocked'], palette: 'warm', w: 4 }),
            ts('Managed and custom rule matches', [sum('azwaf_sec_rule'), sum('azwaf_custom_rule')], { alias: ['managed', 'custom'], w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Network/frontDoors': {
    vars: ['backend', 'httpstatusgroup'],
    monitor: 'azure_front_door',
    note: 'Front Door (classic) profiles: requests by status group at the edge, end-to-end and backend latency, backend health per pool, bytes and the Web Application Firewall’s requests.',
    logs: [['Front Door logs', 'source:azure.network']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Profiles running'),
          qv('Requests', sum('request_count')),
          qv('Server errors (5xx)', sum('request_count', null, 'httpstatusgroup:5xx'), { bad: true }),
          qv('Avg total latency', avg('total_latency'), { precision: 1 }),
          qv('Backend health', avg('backend_health_percentage'), { precision: 1 }),
        ],
        groups: [
          g('Edge requests', [
            ts('Requests by status group', sum('request_count', 'httpstatusgroup'), { display: 'bars', w: 6 }),
            rate('5xx rate by profile', sum('request_count', 'name', 'httpstatusgroup:5xx'), sum('request_count', 'name'), { w: 6 }),
            ts('Backend requests by backend', sum('backend_request_count', 'backend'), { w: 6 }),
            top('Busiest backends', sum('backend_request_count', 'backend'), { w: 6 }),
          ]),
          g('Latency at the edge and the backends', [
            ts('Total latency by profile', avg('total_latency', 'name'), { w: 4 }),
            ts('Backend latency by backend', avg('backend_request_latency', 'backend'), { w: 4 }),
            top('Slowest backends', avg('backend_request_latency', 'backend'), { w: 4 }),
          ]),
          g('Backend pools', [
            ts('Backend health', avg('backend_health_percentage', 'backend'), { w: 8 }),
            top('Least healthy backends', min('backend_health_percentage', 'backend'), { order: 'asc', w: 4 }),
          ]),
          g('Bytes and WAF', [
            mirror('Request and response bytes', sum('request_size'), sum('response_size'), { w: 4 }),
            ts('Billable response bytes', sum('billable_response_size', 'name'), { w: 4 }),
            ts('WAF requests', sum('web_application_firewall_request_count', 'name'), { palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Network/dnszones': {
    vars: [],
    monitor: 'azure_dns',
    note: 'Public DNS zones: the queries they answer (hourly) and their record sets against the zone’s limit.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Zones'),
          qv('Queries (week)', sum('query_volume'), { span: '1w' }),
          qv('Record sets', sum('record_set_count'), { agg: 'last', span: '1d' }),
          qv('Fullest zone', max('record_set_capacity_utilization.max'), { agg: 'last', span: '1d', precision: 1 }),
        ],
        groups: [
          g('Queries', [
            ts('Queries by zone', sum('query_volume', 'name'), { display: 'bars', span: '1w', w: 8 }),
            top('Most queried zones', sum('query_volume', 'name'), { span: '1w', w: 4 }),
            chg('Queries against the week before', sum('query_volume', 'name'), { w: 12 }),
          ]),
          g('Record sets', [
            ts('Record sets by zone', avg('record_set_count', 'name'), { span: '1w' }),
            ts('Record set capacity used', max('record_set_capacity_utilization', 'name'), { span: '1w', palette: 'warm' }),
          ]),
          g('Zones', [
            tbl(
              'Zones',
              [
                ['Queries', sum('query_volume', 'name'), 'sum'],
                ['Record sets', avg('record_set_count', 'name'), 'last'],
                ['Capacity used %', max('record_set_capacity_utilization', 'name'), 'last'],
              ],
              { w: 8, h: 2 },
            ),
            pie('By health status', count('status'), { agg: 'last' }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Network/privateDnsZones': {
    vars: [],
    monitor: 'azure_private_dns',
    note: 'Private DNS zones: the queries resolved inside the linked virtual networks, record sets and virtual network links against the zone’s limits.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Zones'),
          qv('Queries', sum('query_volume')),
          qv('Record sets', sum('record_set_count'), { agg: 'last' }),
          qv('Virtual network links', sum('virtual_network_link_count'), { agg: 'last' }),
        ],
        groups: [
          g('Resolution', [ts('Queries by zone', sum('query_volume', 'name'), { display: 'bars', w: 8 }), tree('Share of queries', sum('query_volume', 'name'), { w: 4 })]),
          g('Record sets and capacity', [
            ts('Record sets by zone', avg('record_set_count', 'name')),
            top('Closest to the record set limit', max('record_set_capacity_utilization', 'name'), { palette: 'warm' }),
          ]),
          g('Virtual network links', [
            ts('Links and auto-registration links', [avg('virtual_network_link_count'), avg('virtual_network_with_registration_link_count')], { alias: ['links', 'with auto-registration'], w: 6 }),
            ts('Link capacity used', [max('virtual_network_link_capacity_utilization'), max('virtual_network_with_registration_capacity_utilization')], {
              alias: ['links %', 'registration links %'],
              palette: 'warm',
              w: 6,
            }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Network/natGateways': {
    vars: [],
    monitor: 'azure_nat_gateway',
    note: 'NAT gateways: datapath availability, the bytes and packets they carry out of the subnets, dropped packets and SNAT connections.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Gateways running'),
          qv('Datapath availability', avg('datapath_availability'), { precision: 2 }),
          qv('Bytes', sum('byte_count')),
          qv('Dropped packets', sum('packet_drop_count'), { bad: true }),
          qv('SNAT connections', sum('snatconnection_count')),
        ],
        groups: [
          g('Datapath', [ts('Availability by gateway', avg('datapath_availability', 'name'), { w: 8 }), top('Least available gateways', min('datapath_availability', 'name'), { order: 'asc', w: 4 })]),
          g('Outbound traffic', [
            ts('Bytes by gateway', sum('byte_count', 'name'), { display: 'bars', w: 4 }),
            ts('Packets by gateway', sum('packet_count', 'name'), { w: 4 }),
            ts('Dropped packets', sum('packet_drop_count', 'name'), { display: 'bars', palette: 'warm', w: 4 }),
          ]),
          g('SNAT', [ts('SNAT connections by gateway', sum('snatconnection_count', 'name')), top('Most connections', sum('total_connection_count', 'name'))]),
        ],
      };
    },
  },

  'Microsoft.Network/publicIPAddresses': {
    vars: [],
    monitor: 'azure_public_ip',
    note: 'Public IP addresses: data path availability, the traffic they receive, and DDoS Protection’s view of it (whether an address is under attack, what it forwards and drops).',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Addresses'),
          qv('Data path availability', avg('vip_availability'), { precision: 2 }),
          qv('Bytes', sum('byte_count')),
          qv('Under DDoS attack', max('if_under_ddo_sattack'), { agg: 'max', bad: true }),
          qv('Packets dropped (DDoS)', max('packets_dropped_ddo_s'), { agg: 'max' }),
        ],
        groups: [
          g('Reachability', [ts('Availability by address', avg('vip_availability', 'name'), { w: 8 }), top('Least available addresses', min('vip_availability', 'name'), { order: 'asc', w: 4 })]),
          g('Inbound traffic', [
            ts('Bytes by address', sum('byte_count', 'name'), { display: 'bars', w: 4 }),
            ts('Packets', sum('packet_count', 'name'), { w: 4 }),
            ts('SYN packets', sum('syn_count', 'name'), { w: 4 }),
          ]),
          g('DDoS Protection', [
            ts('Under attack', max('if_under_ddo_sattack', 'name'), { display: 'bars', palette: 'warm', w: 4 }),
            ts('Inbound packets: forwarded and dropped', [max('packets_forwarded_ddo_s'), max('packets_dropped_ddo_s')], { alias: ['forwarded', 'dropped'], w: 4 }),
            ts('Mitigation triggers (packets/s)', [max('ddo_strigger_tcppackets'), max('ddo_strigger_udppackets'), max('ddo_strigger_synpackets')], { alias: ['TCP', 'UDP', 'SYN'], w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Network/virtualNetworkGateways': {
    vars: [],
    monitor: 'azure_vpn_gateway',
    note: 'Virtual network gateways (VPN and ExpressRoute): site-to-site tunnel bandwidth and drops, BGP peers and routes, point-to-site connections, and the ExpressRoute gateways’ CPU and throughput.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Gateways running'),
          qv('S2S bandwidth', avg('average_bandwidth'), { precision: 0 }),
          qv('Tunnel bytes in', sum('tunnel_ingress_bytes')),
          qv('Dropped tunnel packets', [sum('tunnel_ingress_packet_drop_count'), sum('tunnel_egress_packet_drop_count')], { formula: 'query1 + query2', bad: true }),
          qv('P2S connections', sum('p2sconnection_count'), { agg: 'avg' }),
        ],
        groups: [
          g('Site-to-site tunnels', [
            ts('Tunnel bandwidth', avg('tunnel_average_bandwidth', 'name'), { w: 4 }),
            mirror('Tunnel bytes', sum('tunnel_ingress_bytes'), sum('tunnel_egress_bytes'), { w: 4 }),
            ts('Peak packets per second', max('tunnel_peak_packets', 'name'), { w: 4 }),
          ]),
          g('Dropped packets', [
            ts('Ingress and egress drops', [sum('tunnel_ingress_packet_drop_count'), sum('tunnel_egress_packet_drop_count')], { alias: ['ingress', 'egress'], palette: 'warm', w: 6 }),
            ts('Traffic selector mismatches', [sum('tunnel_ingress_packet_drop_tsmismatch'), sum('tunnel_egress_packet_drop_tsmismatch')], { alias: ['ingress', 'egress'], palette: 'warm', w: 6 }),
          ]),
          g('BGP', [
            ts('BGP peer status', min('bgp_peer_status', 'name'), { w: 4 }),
            ts('Routes learned', sum('bgp_routes_learned', 'name'), { w: 4 }),
            ts('Routes advertised', sum('bgp_routes_advertised', 'name'), { w: 4 }),
          ]),
          g('Point-to-site and ExpressRoute', [
            ts('P2S bandwidth and connections', [avg('p2sbandwidth'), sum('p2sconnection_count')], { alias: ['bandwidth', 'connections'], w: 4 }),
            ts('ExpressRoute gateway CPU', max('express_route_gateway_cpu_utilization', 'name'), { w: 4 }),
            ts('ExpressRoute gateway bits/s', avg('express_route_gateway_bits_per_second', 'name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Network/expressRouteCircuits': {
    vars: [],
    monitor: 'azure_express_route',
    note: 'ExpressRoute circuits: BGP and ARP availability of the peerings, bits in and out against the circuit’s bandwidth, QoS drops and Global Reach traffic.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Circuits'),
          qv('BGP availability', avg('bgp_availability'), { precision: 2 }),
          qv('ARP availability', avg('arp_availability'), { precision: 2 }),
          qv('Peak ingress utilization', max('ingress_bandwidth_utilization'), { precision: 1 }),
          qv('Peak egress utilization', max('egress_bandwidth_utilization'), { precision: 1 }),
        ],
        groups: [
          g('Peering availability', [ts('BGP availability', min('bgp_availability', 'name')), ts('ARP availability', min('arp_availability', 'name'))]),
          g('Bandwidth', [
            mirror('Bits in and out', avg('bits_in_per_second'), avg('bits_out_per_second'), { w: 4 }),
            ts('Ingress utilization', max('ingress_bandwidth_utilization', 'name'), { w: 4 }),
            ts('Egress utilization', max('egress_bandwidth_utilization', 'name'), { w: 4 }),
          ]),
          g('Drops and Global Reach', [
            ts('QoS drops (bits/s)', [avg('qos_drop_bits_in_per_second'), avg('qos_drop_bits_out_per_second')], { alias: ['in', 'out'], palette: 'warm' }),
            mirror('Global Reach bits', avg('global_reach_bits_in_per_second'), avg('global_reach_bits_out_per_second')),
          ]),
        ],
      };
    },
  },

  'Microsoft.Network/azureFirewalls': {
    vars: [],
    monitor: 'azure_firewall',
    note: 'Azure Firewall: health state, latency probe, the application and network rules traffic hits, data processed and throughput, SNAT port use and the capacity units observed.',
    logs: [['Firewall logs', 'source:azure.network']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Firewalls running'),
          qv('Health', avg('firewall_health'), { precision: 1 }),
          qv('Throughput', avg('throughput')),
          qv('Peak SNAT port use', max('snatport_utilization.max'), { precision: 1 }),
          qv('Avg latency probe', avg('firewall_latency_png'), { precision: 2 }),
        ],
        groups: [
          g('Firewall health', [ts('Health state', min('firewall_health', 'name'), { w: 6 }), ts('Latency probe', avg('firewall_latency_png', 'name'), { w: 6 })]),
          g('Rule hits', [
            ts('Application rule hits', sum('application_rule_hit', 'name'), { display: 'bars', w: 6 }),
            ts('Network rule hits', sum('network_rule_hit', 'name'), { display: 'bars', w: 6 }),
          ]),
          g('Traffic and capacity', [
            ts('Data processed', sum('data_processed', 'name'), { w: 4 }),
            ts('SNAT port utilization', max('snatport_utilization', 'name'), { palette: 'warm', w: 4 }),
            ts('Observed capacity units', avg('observed_capacity', 'name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.EventHub/namespaces': {
    vars: ['entityname'],
    monitor: 'azure_event_hub',
    note: 'Event Hubs namespaces: messages and bytes in and out per event hub, throttled requests and errors, connections, Capture and geo-replication lag, and the namespace’s CPU and memory.',
    logs: [['Event Hubs logs', 'source:azure.eventhub']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Namespaces running'),
          qv('Incoming messages', sum('incoming_messages')),
          qv('Outgoing messages', sum('outgoing_messages')),
          qv('Throttled requests', sum('throttled_requests'), { bad: true }),
          qv('Server errors', sum('server_errors'), { bad: true }),
        ],
        groups: [
          g('Messages per event hub', [
            ts('Incoming by event hub', sum('incoming_messages', 'entityname'), { display: 'bars', w: 4 }),
            ts('Outgoing by event hub', sum('outgoing_messages', 'entityname'), { display: 'bars', w: 4 }),
            ts('Consumed against produced', [sum('outgoing_messages', 'entityname'), sum('incoming_messages', 'entityname')], { formulas: [['100 * query1 / query2', 'out / in %']], w: 4 }),
          ]),
          g('Bytes and size', [mirror('Bytes in and out', sum('incoming_bytes'), sum('outgoing_bytes'), { w: 6 }), top('Largest event hubs', max('size', 'entityname'), { w: 6 })]),
          g('Throttling and errors', [
            ts('Throttled by event hub', sum('throttled_requests', 'entityname'), { palette: 'warm', w: 4 }),
            ts('Server and user errors', [sum('server_errors'), sum('user_errors')], { alias: ['server', 'user'], palette: 'warm', w: 4 }),
            ts('Quota exceeded', sum('quota_exceeded_errors', 'entityname'), { palette: 'warm', w: 4 }),
          ]),
          g('Connections', [
            ts('Active connections', max('active_connections', 'name'), { w: 6 }),
            ts('Opened and closed', [sum('connections_opened'), sum('connections_closed')], { alias: ['opened', 'closed'], w: 6 }),
          ]),
          g('Capture and replication', [
            ts('Capture backlog', sum('capture_backlog', 'entityname'), { w: 4 }),
            ts('Captured messages', sum('captured_messages', 'entityname'), { w: 4 }),
            ts('Geo-replication lag', max('replication_lag_duration', 'entityname'), { palette: 'warm', w: 4 }),
          ]),
          g('Namespace resources', [ts('CPU', max('namespace_cpu_usage', 'name')), ts('Memory', max('namespace_memory_usage', 'name'))]),
        ],
      };
    },
  },

  'Microsoft.ServiceBus/namespaces': {
    vars: ['entityname'],
    monitor: 'azure_service_bus',
    note: 'Service Bus namespaces: active, dead-lettered and scheduled messages per queue and topic, messages in, out, completed and abandoned, errors and throttling, send latency and size.',
    logs: [['Service Bus logs', 'source:azure.servicebus']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Namespaces running'),
          qv('Active messages', sum('active_messages'), { agg: 'last' }),
          qv('Dead-lettered', sum('deadlettered_messages'), { agg: 'last', bad: true }),
          qv('Incoming messages', sum('incoming_messages')),
          qv('Throttled requests', sum('throttled_requests'), { bad: true }),
        ],
        groups: [
          g('Queue and topic depth', [
            ts('Active messages', avg('active_messages', 'entityname'), { w: 4 }),
            ts('Dead-lettered messages', avg('deadlettered_messages', 'entityname'), { palette: 'warm', w: 4 }),
            top('Deepest entities', max('active_messages', 'entityname'), { w: 4 }),
            ts('Scheduled messages', avg('scheduled_messages', 'entityname'), { w: 12 }),
          ]),
          g('Message flow', [
            ts('Incoming and outgoing', [sum('incoming_messages'), sum('outgoing_messages')], { alias: ['incoming', 'outgoing'], w: 6 }),
            ts('Completed and abandoned', [sum('complete_message'), sum('abandon_message')], { alias: ['completed', 'abandoned'], w: 6 }),
          ]),
          g('Errors and throttling', [
            ts('Server errors by entity', sum('server_errors', 'entityname'), { palette: 'warm', w: 4 }),
            ts('User errors by entity', sum('user_errors', 'entityname'), { palette: 'warm', w: 4 }),
            ts('Throttled requests', sum('throttled_requests', 'entityname'), { palette: 'warm', w: 4 }),
          ]),
          g('Latency and size', [ts('Server send latency', avg('server_send_latency', 'entityname'), { w: 6 }), ts('Size by entity', max('size', 'entityname'), { w: 6 })]),
          g('Namespace', [
            ts('Active connections', sum('active_connections', 'name'), { w: 4 }),
            ts('CPU', max('namespace_cpu_usage.max', 'name'), { w: 4 }),
            ts('Memory', max('namespace_memory_usage.max', 'name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.EventGrid/topics': {
    vars: ['eventsubscriptionname'],
    monitor: 'azure_event_grid',
    note: 'Custom Event Grid topics: events published and failed, matched to subscriptions or not, delivered or failed per subscription, dropped and dead-lettered events.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Topics running'),
          qv('Published', sum('publish_success_count')),
          qv('Publish failures', sum('publish_fail_count'), { bad: true }),
          qv('Delivered', sum('delivery_success_count')),
          qv('Dead-lettered', sum('dead_lettered_count'), { bad: true }),
        ],
        groups: [
          g('Publishing', [
            ts('Published by topic', sum('publish_success_count', 'name'), { display: 'bars', w: 4 }),
            ts('Publish failures', sum('publish_fail_count', 'name'), { palette: 'warm', w: 4 }),
            ts('Publish latency', avg('publish_success_latency_in_ms', 'name'), { w: 4 }),
          ]),
          g('Delivery per subscription', [
            ts('Delivered', sum('delivery_success_count', 'eventsubscriptionname'), { w: 4 }),
            ts('Failed delivery attempts', sum('delivery_attempt_fail_count', 'eventsubscriptionname'), { palette: 'warm', w: 4 }),
            ts('Destination processing time', avg('destination_processing_duration_in_ms', 'eventsubscriptionname'), { w: 4 }),
          ]),
          g('Matching, drops and dead letters', [
            ts('Matched and unmatched', [sum('matched_event_count'), sum('unmatched_event_count')], { alias: ['matched', 'unmatched'], w: 4 }),
            ts('Dropped events', sum('dropped_event_count', 'eventsubscriptionname'), { palette: 'warm', w: 4 }),
            ts('Dead-lettered events', sum('dead_lettered_count', 'eventsubscriptionname'), { palette: 'warm', w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.EventGrid/systemTopics': {
    vars: ['eventsubscriptionname'],
    monitor: 'azure_event_grid',
    note: 'System topics: the events Azure services (storage accounts, resource groups, Key Vault, …) publish, and how each subscription receives them.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'System topics'),
          qv('Events published', sum('publish_success_count')),
          qv('Delivered', sum('delivery_success_count')),
          qv('Failed deliveries', sum('delivery_attempt_fail_count'), { bad: true }),
          qv('Dropped', sum('dropped_event_count'), { bad: true }),
        ],
        groups: [
          g('Events from Azure services', [
            ts('Events by system topic', sum('publish_success_count', 'name'), { display: 'bars', w: 8 }),
            tree('Share of events', sum('publish_success_count', 'name'), { w: 4 }),
          ]),
          g('Subscriptions', [
            ts('Delivered by subscription', sum('delivery_success_count', 'eventsubscriptionname'), { w: 6 }),
            top('Slowest destinations', avg('destination_processing_duration_in_ms', 'eventsubscriptionname'), { w: 6 }),
          ]),
          g('Failures', [
            ts('Failed delivery attempts', sum('delivery_attempt_fail_count', 'eventsubscriptionname'), { palette: 'warm', w: 4 }),
            ts('Dropped', sum('dropped_event_count', 'eventsubscriptionname'), { palette: 'warm', w: 4 }),
            ts('Dead-lettered', sum('dead_lettered_count', 'eventsubscriptionname'), { palette: 'warm', w: 4 }),
          ]),
          g('Filtering', [
            ts('Matched and unmatched events', [sum('matched_event_count'), sum('unmatched_event_count')], { alias: ['matched', 'unmatched'] }),
            ts('Advanced filter evaluations', sum('advanced_filter_evaluation_count', 'eventsubscriptionname')),
          ]),
        ],
      };
    },
  },

  'Microsoft.ApiManagement/service': {
    vars: ['apiid', 'location', 'gatewayresponsecodecategory'],
    monitor: 'azure_api_management',
    note: 'API Management instances: gateway requests by response class and API, gateway and backend duration, capacity per region, and the events logged to Event Hubs.',
    logs: [['API Management logs', 'source:azure.apimanagement']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Instances running'),
          qv('Requests', sum('requests')),
          qv('Gateway 5xx', sum('requests', null, 'gatewayresponsecodecategory:5xx'), { bad: true }),
          qv('Avg duration', avg('duration'), { precision: 0 }),
          qv('Peak capacity', max('capacity.max'), { precision: 1 }),
        ],
        groups: [
          g('Gateway requests', [
            ts('Requests by response class', sum('requests', 'gatewayresponsecodecategory'), { display: 'bars', w: 6 }),
            ts('Requests by API', sum('requests', 'apiid'), { w: 6 }),
            top('Busiest APIs', sum('requests', 'apiid'), { w: 6 }),
            top('APIs with the most 5xx', sum('requests', 'apiid', 'gatewayresponsecodecategory:5xx'), { palette: 'warm', w: 6 }),
          ]),
          g('Gateway and backend duration', [
            ts('Overall and backend duration', [avg('duration'), avg('backend_duration')], { alias: ['overall', 'backend'], w: 4 }),
            ts('Time in the gateway', [avg('duration', 'apiid'), avg('backend_duration', 'apiid')], { formulas: [['query1 - query2', 'gateway']], w: 4 }),
            top('Slowest APIs', avg('duration', 'apiid'), { w: 4 }),
          ]),
          g('Capacity', [
            ts('Capacity by region', max('capacity', 'location'), { palette: 'warm', w: 4 }),
            ts('Gateway CPU', max('cpu_percent_gateway', 'name'), { w: 4 }),
            ts('Gateway memory', max('memory_percent_gateway', 'name'), { w: 4 }),
          ]),
          g('Event Hub logging', [
            ts('Events logged', [sum('event_hub_successful_events'), sum('event_hub_total_failed_events')], { alias: ['successful', 'failed'] }),
            ts('Dropped, rejected and throttled', [sum('event_hub_dropped_events'), sum('event_hub_rejected_events'), sum('event_hub_throttled_events')], {
              alias: ['dropped', 'rejected', 'throttled'],
              palette: 'warm',
            }),
          ]),
        ],
      };
    },
  },

  'Microsoft.ContainerRegistry/registries': {
    vars: [],
    monitor: 'azure_container_registry',
    note: 'Container registries: image pulls and pushes and how many succeed, storage used, data transferred and ACR Tasks runs.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Registries'),
          qv('Pulls', sum('total_pull_count')),
          qv('Failed pulls', [sum('total_pull_count'), sum('successful_pull_count')], { formula: 'query1 - query2', bad: true }),
          qv('Pushes', sum('total_push_count')),
          qv('Storage used', sum('storage_used'), { agg: 'last', span: '1w' }),
        ],
        groups: [
          g('Pulls and pushes', [
            ts('Pulls and pushes', [sum('total_pull_count'), sum('total_push_count')], { alias: ['pulls', 'pushes'], display: 'bars', w: 4 }),
            ts('Pull success rate', [sum('successful_pull_count', 'name'), sum('total_pull_count', 'name')], { formulas: [['100 * query1 / query2', 'success %']], w: 4 }),
            top('Most pulled registries', sum('total_pull_count', 'name'), { w: 4 }),
          ]),
          g('Registry storage', [
            ts('Storage used', avg('storage_used', 'name'), { display: 'area', span: '1w', w: 8 }),
            chg('Against the week before', avg('storage_used', 'name'), { increaseGood: false, w: 4 }),
          ]),
          g('Transfer and tasks', [
            ts('Data transferred', sum('data_transfer', 'name'), { w: 4 }),
            ts('Task run duration', sum('run_duration', 'name'), { w: 4 }),
            ts('Agent pool CPU time', sum('agent_pool_cputime', 'name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.App/containerApps': {
    vars: ['revisionname', 'statuscodecategory'],
    monitor: 'azure_container_apps',
    note: 'Container apps: requests by status class and revision, response time, replicas and restarts, CPU and memory of the replicas, network and the resiliency policies’ retries and timeouts.',
    logs: [['Container Apps logs', 'source:azure.app']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Apps running'),
          qv('Requests', sum('requests')),
          qv('Server errors (5xx)', sum('requests', null, 'statuscodecategory:5xx'), { bad: true }),
          qv('Replicas', sum('replicas'), { agg: 'last' }),
          qv('Restarts', sum('restart_count'), { agg: 'max', bad: true }),
        ],
        groups: [
          g('Requests by revision', [
            ts('Requests by status class', sum('requests', 'statuscodecategory'), { display: 'bars', w: 4 }),
            ts('Requests by revision', sum('requests', 'revisionname'), { w: 4 }),
            ts('Response time', avg('response_time', 'name'), { w: 4 }),
          ]),
          g('Replicas', [
            ts('Replicas by revision', avg('replicas', 'revisionname'), { w: 4 }),
            ts('Restarts', max('restart_count', 'name'), { palette: 'warm', w: 4 }),
            ts('Reserved cores', max('cores_quota_used', 'revisionname'), { w: 4 }),
          ]),
          g('Replica resources', [
            ts('CPU (nanocores)', sum('usage_nano_cores', 'name'), { w: 4 }),
            ts('Memory working set', sum('working_set_bytes', 'name'), { w: 4 }),
            mirror('Network', avg('rx_bytes'), avg('tx_bytes'), { w: 4 }),
          ]),
          g('Resiliency', [
            ts('Retries and timeouts', [sum('resiliency_request_retries'), sum('resiliency_request_timeouts'), sum('resiliency_connect_timeouts')], {
              alias: ['retries', 'request timeouts', 'connect timeouts'],
              palette: 'warm',
            }),
            ts('Ejected hosts', sum('resiliency_ejected_hosts', 'name'), { palette: 'warm' }),
          ]),
        ],
      };
    },
  },

  'Microsoft.ContainerInstance/containerGroups': {
    vars: [],
    monitor: 'azure_container_instances',
    note: 'Container groups: the CPU (millicores) and memory their containers use and the bytes they send and receive.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [...health(count, 'Groups running'), qv('Peak CPU (millicores)', max('cpu_usage'), { agg: 'max' }), qv('Peak memory', max('memory_usage'), { agg: 'max' })],
        groups: [
          g('CPU', [ts('CPU by group (millicores)', max('cpu_usage', 'name'), { w: 8 }), top('Busiest groups', max('cpu_usage', 'name'), { w: 4 })]),
          g('Memory', [ts('Memory by group', max('memory_usage', 'name'), { w: 8 }), top('Largest groups', max('memory_usage', 'name'), { w: 4 })]),
          g('Network', [mirror('Bytes per second', max('network_bytes_received_per_second'), max('network_bytes_transmitted_per_second'), { w: 12 })]),
        ],
      };
    },
  },

  'Microsoft.OperationalInsights/workspaces': {
    vars: [],
    monitor: 'azure_log_analytics',
    note: 'Log Analytics workspaces: how much they ingest and how long it takes, the queries run and failing, data export, and the heartbeats of the agents reporting to them.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Workspaces'),
          qv('Ingestion volume', sum('ingestion_volume')),
          qv('Avg ingestion time', avg('ingestion_time'), { precision: 1 }),
          qv('Queries', sum('query_count')),
          qv('Failed queries', sum('query_failure_count'), { bad: true }),
        ],
        groups: [
          g('Ingestion', [
            ts('Volume by workspace', sum('ingestion_volume', 'name'), { display: 'bars', w: 4 }),
            ts('Ingestion time (avg and max)', [avg('ingestion_time'), max('ingestion_time.max')], { alias: ['avg', 'max'], w: 4 }),
            top('Largest workspaces', sum('ingestion_volume', 'name'), { w: 4 }),
          ]),
          g('Queries', [
            ts('Queries and failures', [sum('query_count'), sum('query_failure_count')], { alias: ['queries', 'failed'], w: 6 }),
            ts('Query availability', min('availability_rate_query', 'name'), { w: 6 }),
          ]),
          g('Data export', [
            ts('Records exported', sum('records_exported', 'name'), { w: 4 }),
            ts('Bytes exported', sum('bytes_exported', 'name'), { w: 4 }),
            ts('Failed exports', sum('export_failed', 'name'), { palette: 'warm', w: 4 }),
          ]),
          g('Agents', [ts('Heartbeats', sum('heartbeat', 'name'), { display: 'bars', w: 12 })]),
        ],
      };
    },
  },

  'Microsoft.Insights/components': {
    vars: [],
    monitor: 'azure_application_insights',
    note: 'Application Insights resources: server requests, failures and response time, dependency calls, exceptions, availability tests, page views and the apps’ performance counters, as Application Insights aggregates them.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Resources'),
          qv('Server requests', sum('requests_count')),
          qv('Failed requests', sum('requests_failed'), { bad: true }),
          qv('Avg response time', avg('requests_duration'), { precision: 0 }),
          qv('Exceptions', sum('exceptions_count'), { bad: true }),
        ],
        groups: [
          g('Server requests', [
            ts('Requests and failures', [sum('requests_count'), sum('requests_failed')], { alias: ['requests', 'failed'], display: 'bars', w: 4 }),
            rate('Failure rate', sum('requests_failed', 'name'), sum('requests_count', 'name'), { w: 4 }),
            ts('Response time', avg('requests_duration', 'name'), { w: 4 }),
          ]),
          g('Dependencies', [
            ts('Dependency calls and failures', [sum('dependencies_count'), sum('dependencies_failed')], { alias: ['calls', 'failed'], w: 6 }),
            ts('Dependency duration', avg('dependencies_duration', 'name'), { w: 6 }),
          ]),
          g('Exceptions and traces', [
            ts('Server and browser exceptions', [sum('exceptions_server'), sum('exceptions_browser')], { alias: ['server', 'browser'], palette: 'warm', w: 6 }),
            ts('Traces', sum('traces_count', 'name'), { w: 6 }),
          ]),
          g('Availability tests', [
            ts('Availability', avg('availability_results_availability_percentage', 'name'), { w: 6 }),
            ts('Test duration', avg('availability_results_duration', 'name'), { w: 6 }),
          ]),
          g('Browser', [ts('Page views', sum('page_views_count', 'name'), { w: 6 }), ts('Page load time', avg('browser_timings_total_duration', 'name'), { w: 6 })]),
          g('Performance counters', [
            ts('Process CPU', avg('performance_counters_process_cpu_percentage', 'name'), { w: 4 }),
            ts('Available memory', avg('performance_counters_memory_available_bytes', 'name'), { w: 4 }),
            ts('Requests in the application queue', avg('performance_counters_requests_in_queue', 'name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.RecoveryServices/vaults': {
    vars: ['healthstatus', 'datasourcetype'],
    monitor: 'azure_backup',
    note: 'Recovery Services vaults (Azure Backup): the health events of their backup and restore jobs, by health state and by workload (VMs, SQL in VMs, file shares, …).',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      const bad = '!healthstatus:healthy';
      return {
        kpis: [
          ...health(count, 'Vaults'),
          qv('Backup health events', sum('backup_health_event')),
          qv('Unhealthy backups', sum('backup_health_event', null, bad), { bad: true }),
          qv('Restore health events', sum('restore_health_event')),
          qv('Unhealthy restores', sum('restore_health_event', null, bad), { bad: true }),
        ],
        groups: [
          g('Backup jobs', [
            ts('Backup health by state', sum('backup_health_event', 'healthstatus'), { display: 'bars', w: 4 }),
            ts('Backup health by workload', sum('backup_health_event', 'datasourcetype'), { w: 4 }),
            top('Vaults with unhealthy backups', sum('backup_health_event', 'name', bad), { palette: 'warm', w: 4 }),
          ]),
          g('Restore jobs', [
            ts('Restore health by state', sum('restore_health_event', 'healthstatus'), { display: 'bars' }),
            top('Workloads with unhealthy restores', sum('restore_health_event', 'datasourcetype', bad), { palette: 'warm' }),
          ]),
          g('Vaults', [
            tbl(
              'Vaults',
              [
                ['Backup events', sum('backup_health_event', 'name'), 'sum'],
                ['Unhealthy backups', sum('backup_health_event', 'name', bad), 'sum'],
                ['Restore events', sum('restore_health_event', 'name'), 'sum'],
              ],
              { w: 8, h: 2 },
            ),
            pie('By health status', count('status'), { agg: 'last' }),
          ]),
        ],
      };
    },
  },

  'Microsoft.DataProtection/backupVaults': {
    vars: ['healthstatus', 'datasourcetype'],
    monitor: 'azure_backup_vault',
    note: 'Backup vaults (Azure Backup for disks, blobs, AKS and PostgreSQL): the health of their backup and restore jobs per workload type and backup instance state.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      const bad = '!healthstatus:healthy';
      return {
        kpis: [
          ...health(count, 'Backup vaults'),
          qv('Backup jobs', sum('backup_health_event')),
          qv('Failing backups', sum('backup_health_event', null, bad), { bad: true }),
          qv('Restore jobs', sum('restore_health_event')),
          qv('Failing restores', sum('restore_health_event', null, bad), { bad: true }),
        ],
        groups: [
          g('Workloads', [
            pie('Backups by workload', sum('backup_health_event', 'datasourcetype')),
            ts('Failing backups by workload', sum('backup_health_event', 'datasourcetype', bad), { display: 'bars', palette: 'warm', w: 8 }),
          ]),
          g('Backup instance health', [
            ts('Backups by health state', sum('backup_health_event', 'healthstatus'), { display: 'area' }),
            top('Vaults with failing backups', sum('backup_health_event', 'name', bad), { palette: 'warm' }),
          ]),
          g('Restores', [ts('Restores by health state', sum('restore_health_event', 'healthstatus'), { display: 'bars' }), ts('Restores by workload', sum('restore_health_event', 'datasourcetype'))]),
        ],
      };
    },
  },

  'Microsoft.CognitiveServices/accounts': {
    vars: ['modeldeploymentname', 'statuscode'],
    monitor: 'azure_openai',
    note: 'Azure OpenAI and AI Services accounts: calls and their errors, prompt and completion tokens per model deployment, model requests by status code, latency and time to first token, provisioned utilization and content filtering.',
    logs: [['AI Services logs', 'source:azure.cognitiveservices']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Accounts running'),
          qv('Calls', sum('total_calls')),
          qv('Errors', sum('total_errors'), { bad: true }),
          qv('Tokens processed', sum('token_transaction')),
          qv('Avg latency', avg('latency'), { precision: 0 }),
        ],
        groups: [
          g('Calls', [
            ts('Calls by account', sum('total_calls', 'name'), { display: 'bars', w: 4 }),
            ts('Client and server errors', [sum('client_errors'), sum('server_errors')], { alias: ['client', 'server'], palette: 'warm', w: 4 }),
            ts('Rate limited and blocked', [sum('ratelimit'), sum('blocked_calls')], { alias: ['rate limit', 'blocked'], palette: 'warm', w: 4 }),
          ]),
          g('Tokens by model deployment', [
            ts('Prompt and completion tokens', [sum('processed_prompt_tokens'), sum('generated_tokens')], { alias: ['prompt', 'completion'], display: 'bars', w: 4 }),
            ts('Tokens by deployment', sum('token_transaction', 'modeldeploymentname'), { w: 4 }),
            pie('Share of tokens', sum('token_transaction', 'modeldeploymentname')),
          ]),
          g('Model requests', [
            ts('Requests by status code', sum('azure_open_airequests', 'statuscode'), { display: 'bars', w: 6 }),
            top('Busiest deployments', sum('azure_open_airequests', 'modeldeploymentname'), { w: 6 }),
          ]),
          g('Latency', [
            ts('Latency by account', avg('latency', 'name'), { w: 4 }),
            ts('Time to first token', max('azure_open_ainormalized_ttftin_ms', 'modeldeploymentname'), { w: 4 }),
            ts('Tokens per second', max('azure_open_aitoken_per_second', 'modeldeploymentname'), { w: 4 }),
          ]),
          g('Provisioned throughput and safety', [
            ts('Provisioned utilization', max('azure_open_aiprovisioned_managed_utilization_v2.max', 'modeldeploymentname'), { palette: 'warm', w: 6 }),
            ts('Content filtering', [sum('raitotal_requests'), sum('rairejected_requests'), sum('raiharmful_requests')], { alias: ['checked', 'blocked', 'harmful'], w: 6 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Search/searchServices': {
    vars: [],
    monitor: 'azure_search',
    note: 'AI Search services: queries per second, search latency and the share of queries throttled, documents and skills the indexers process, and index storage.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Services running'),
          qv('Queries per second', avg('search_queries_per_second'), { precision: 1 }),
          qv('Avg search latency', avg('search_latency'), { precision: 3 }),
          qv('Throttled queries', max('throttled_search_queries_percentage'), { precision: 1, bad: true }),
          qv('Documents processed', sum('documents_processed_count')),
        ],
        groups: [
          g('Search queries', [
            ts('Queries per second', avg('search_queries_per_second', 'name'), { w: 4 }),
            ts('Search latency', avg('search_latency', 'name'), { w: 4 }),
            ts('Throttled queries (%)', max('throttled_search_queries_percentage', 'name'), { palette: 'warm', w: 4 }),
          ]),
          g('Indexers', [
            ts('Documents processed', sum('documents_processed_count', 'name'), { display: 'bars', w: 4 }),
            ts('Bytes processed', sum('documents_processed_bytes', 'name'), { w: 4 }),
            ts('Skill executions', sum('skill_execution_count', 'name'), { w: 4 }),
          ]),
          g('Index storage', [ts('Storage used', avg('index_storage_usage', 'name')), ts('Vector storage used', avg('index_vector_usage', 'name'))]),
        ],
      };
    },
  },

  'Microsoft.MachineLearningServices/workspaces': {
    vars: [],
    monitor: 'azure_machine_learning',
    note: 'Machine Learning workspaces: runs started, completed and failed, compute cluster nodes and cores, quota use, model registrations and deployments, errors and warnings.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Workspaces'),
          qv('Completed runs', sum('completed_runs')),
          qv('Failed runs', sum('failed_runs'), { bad: true }),
          qv('Active nodes', sum('active_nodes'), { agg: 'avg' }),
          qv('Failed deployments', sum('model_deploy_failed'), { bad: true }),
        ],
        groups: [
          g('Runs', [
            ts('Started, completed and failed', [sum('started_runs'), sum('completed_runs'), sum('failed_runs')], { alias: ['started', 'completed', 'failed'], display: 'bars', w: 6 }),
            ts('Queued and not responding', [sum('queued_runs'), sum('not_responding_runs')], { alias: ['queued', 'not responding'], palette: 'warm', w: 6 }),
          ]),
          g('Compute clusters', [
            ts('Nodes: active, idle, preempted', [avg('active_nodes'), avg('idle_nodes'), avg('preempted_nodes')], { alias: ['active', 'idle', 'preempted'], w: 4 }),
            ts('Cores in use', avg('active_cores', 'name'), { w: 4 }),
            ts('Quota used', max('quota_utilization_percentage', 'name'), { palette: 'warm', w: 4 }),
          ]),
          g('Models', [
            ts('Deployments', [sum('model_deploy_started'), sum('model_deploy_succeeded'), sum('model_deploy_failed')], { alias: ['started', 'succeeded', 'failed'] }),
            ts('Registrations', [sum('model_register_succeeded'), sum('model_register_failed')], { alias: ['succeeded', 'failed'] }),
          ]),
          g('Errors and utilization', [
            ts('Errors and warnings', [sum('errors'), sum('warnings')], { alias: ['errors', 'warnings'], palette: 'warm', w: 4 }),
            ts('CPU utilization', avg('cpu_utilization', 'name'), { w: 4 }),
            ts('GPU utilization', avg('gpu_utilization', 'name'), { w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Synapse/workspaces': {
    vars: ['result'],
    monitor: 'azure_synapse',
    note: 'Synapse workspaces: pipeline, activity and trigger runs by result, the serverless SQL pool’s data processed and requests, Synapse Link and streaming jobs.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Workspaces'),
          qv('Pipeline runs', sum('integration_pipeline_runs_ended')),
          qv('Failed pipeline runs', sum('integration_pipeline_runs_ended', null, 'result:failed'), { bad: true }),
          qv('Activity runs', sum('integration_activity_runs_ended')),
          qv('Serverless data processed', sum('builtin_sql_pool_data_processed_bytes')),
        ],
        groups: [
          g('Integration runs', [
            ts('Pipeline runs by result', sum('integration_pipeline_runs_ended', 'result'), { display: 'bars', w: 4 }),
            ts('Activity runs by result', sum('integration_activity_runs_ended', 'result'), { w: 4 }),
            ts('Trigger runs by result', sum('integration_trigger_runs_ended', 'result'), { w: 4 }),
          ]),
          g('Serverless SQL pool', [
            ts('Data processed', sum('builtin_sql_pool_data_processed_bytes', 'name'), { w: 4 }),
            ts('Requests ended by result', avg('builtin_sql_pool_requests_ended', 'result'), { w: 4 }),
            ts('Login attempts by result', avg('builtin_sql_pool_login_attempts', 'result'), { w: 4 }),
          ]),
          g('Synapse Link and streaming', [
            ts('Link rows processed', sum('integration_link_processed_changed_rows', 'name'), { w: 4 }),
            ts('Link latency', max('integration_link_processing_latency_in_seconds', 'name'), { w: 4 }),
            ts('Streaming input and output events', [sum('sqlstreaming_input_events'), sum('sqlstreaming_output_events')], { alias: ['input', 'output'], w: 4 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.DataFactory/factories': {
    vars: [],
    monitor: 'azure_data_factory',
    note: 'Data factories: pipeline, activity and trigger runs that succeed, fail or are cancelled, the integration runtimes’ CPU, memory and queues, and the factory against its size and entity limits.',
    logs: [['Data Factory logs', 'source:azure.datafactory']],
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Factories'),
          qv('Succeeded pipeline runs', sum('pipeline_succeeded_runs'), { good: true }),
          qv('Failed pipeline runs', sum('pipeline_failed_runs'), { bad: true }),
          qv('Failed activity runs', sum('activity_failed_runs'), { bad: true }),
          qv('Failed trigger runs', sum('trigger_failed_runs'), { bad: true }),
        ],
        groups: [
          g('Pipeline runs', [
            ts('Succeeded, failed and cancelled', [sum('pipeline_succeeded_runs'), sum('pipeline_failed_runs'), sum('pipeline_cancelled_runs')], {
              alias: ['succeeded', 'failed', 'cancelled'],
              display: 'bars',
              w: 4,
            }),
            ts('Failure rate by factory', [sum('pipeline_failed_runs', 'name'), sum('pipeline_succeeded_runs', 'name')], {
              formulas: [['100 * query1 / (query1 + query2)', 'failed %']],
              palette: 'warm',
              w: 4,
            }),
            top('Factories with the most failures', sum('pipeline_failed_runs', 'name'), { palette: 'warm', w: 4 }),
          ]),
          g('Activities and triggers', [
            ts('Activity runs', [sum('activity_succeeded_runs'), sum('activity_failed_runs'), sum('activity_cancelled_runs')], { alias: ['succeeded', 'failed', 'cancelled'] }),
            ts('Trigger runs', [sum('trigger_succeeded_runs'), sum('trigger_failed_runs'), sum('trigger_cancelled_runs')], { alias: ['succeeded', 'failed', 'cancelled'] }),
          ]),
          g('Integration runtimes', [
            ts('CPU', avg('integration_runtime_cpu_percentage', 'name'), { w: 4 }),
            ts('Available memory', avg('integration_runtime_available_memory', 'name'), { w: 4 }),
            ts('Queue length and wait', [avg('integration_runtime_queue_length'), avg('integration_runtime_average_task_pickup_delay')], { alias: ['queue length', 'queue duration'], w: 4 }),
          ]),
          g('Factory limits', [
            ts('Size used (% of the limit)', [max('factory_size_in_gb_units', 'name'), max('max_allowed_factory_size_in_gb_units', 'name')], { formulas: [['100 * query1 / query2', 'size %']], w: 6 }),
            ts('Entities (% of the limit)', [max('resource_count', 'name'), max('max_allowed_resource_count', 'name')], { formulas: [['100 * query1 / query2', 'entities %']], w: 6 }),
          ]),
        ],
      };
    },
  },

  'Microsoft.Insights/dataCollectionRules': {
    vars: [],
    monitor: 'azure_monitor_data_collection',
    note: 'Data collection rules: the Logs Ingestion API requests and bytes they receive, the rows they keep and drop, their transformations’ errors and duration, and metrics ingestion.',
    build: (q) => {
      const { sum, avg, max, min, count } = on(q);
      return {
        kpis: [
          ...health(count, 'Rules'),
          qv('Ingestion requests', sum('api_call_received_count')),
          qv('Rows received', sum('rows_received_count')),
          qv('Rows dropped', sum('rows_dropped_count'), { bad: true }),
          qv('Transform errors', sum('transformation_errors_count'), { bad: true }),
        ],
        groups: [
          g('Logs ingestion', [
            ts('Requests by rule', sum('api_call_received_count', 'name'), { display: 'bars', w: 4 }),
            ts('Bytes received', sum('bytes_received_count', 'name'), { w: 4 }),
            mirror('Uncompressed bytes in and out', sum('uncompressed_bytes_in'), sum('uncompressed_bytes_out'), { w: 4 }),
          ]),
          g('Rows kept and dropped', [
            ts('Received and dropped', [sum('rows_received_count'), sum('rows_dropped_count')], { alias: ['received', 'dropped'] }),
            ts('Columns dropped', sum('columns_dropped_count', 'name'), { palette: 'warm' }),
          ]),
          g('Transformations', [ts('Transform errors', sum('transformation_errors_count', 'name'), { palette: 'warm' }), ts('Transform duration', avg('transformation_runtime_duration_ms', 'name'))]),
          g('Metrics ingestion', [ts('Metrics ingestion requests', sum('metric_ingestion_request_count', 'name'), { display: 'bars', w: 12 })]),
        ],
      };
    },
  },
};
