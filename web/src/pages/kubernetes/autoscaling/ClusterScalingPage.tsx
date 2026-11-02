// Autoscaling → Cluster Scaling: each cluster's node autoscaler, capacity and what removing idle
// nodes and rightsizing its workloads would save.
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Search } from 'lucide-react';
import { formatBytes } from '../../../lib/format';
import { money } from '../../cloudCost/costModel';
import { useOverview } from './AutoscalingLayout';
import { EmptyResults } from './EmptyResults';

export const NODE_AUTOSCALERS = { karpenter: 'Karpenter', cluster_autoscaler: 'Cluster Autoscaler', none: 'Not scaled' } as const;

export function ClusterScalingPage() {
  const { clusters } = useOverview();
  const [query, setQuery] = useState('');
  const shown = clusters.filter((c) => c.cluster.toLowerCase().includes(query.trim().toLowerCase())).sort((a, b) => b.projectedClusterSavings - a.projectedClusterSavings);
  return (
    <div className="as-body">
      <label className="as-search">
        <Search size={16} />
        <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Filter your clusters" />
      </label>
      <p className="as-count">
        Showing <strong>{shown.length ? `1–${shown.length}` : '0–0'}</strong> of <strong>{clusters.length}</strong> Clusters
      </p>
      <table className="settings-table as-table">
        <thead>
          <tr>
            <th>Projected savings /mo</th>
            <th>Cluster</th>
            <th>Autoscaling</th>
            <th>Nodes</th>
            <th>CPU capacity</th>
            <th>Mem capacity</th>
            <th>CPU requested</th>
            <th>Projected workload savings /mo</th>
          </tr>
        </thead>
        <tbody>
          {shown.map((c) => (
            <tr key={c.cluster}>
              <td className="as-money">{money(c.projectedClusterSavings)}</td>
              <td>
                <Link to={`/orchestration/explorer?resource=nodes&query=${encodeURIComponent(`kube_cluster_name:${c.cluster}`)}`}>{c.cluster}</Link>
              </td>
              <td>
                <span className={`as-pill ${c.autoscaling === 'none' ? 'as-pill-off' : 'as-pill-on'}`}>{NODE_AUTOSCALERS[c.autoscaling]}</span>
              </td>
              <td>{c.nodes}</td>
              <td>{c.cpuCapacity.toFixed(1)} cores</td>
              <td>{formatBytes(c.memoryCapacity)}</td>
              <td>{c.cpuCapacity ? `${Math.round((c.cpuRequests / c.cpuCapacity) * 100)}%` : '—'}</td>
              <td className="as-money">{money(c.projectedWorkloadSavings)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {shown.length === 0 && (
        <EmptyResults title="No Results Found">
          {clusters.length ? 'No cluster matches the filter.' : 'Deploy the Datadog Agent with the orchestrator explorer to collect your clusters’ Kubernetes resources.'}{' '}
          {!clusters.length && <Link to="/integrations/kubernetes?tab=install">Set up a cluster</Link>}
        </EmptyResults>
      )}
    </div>
  );
}
