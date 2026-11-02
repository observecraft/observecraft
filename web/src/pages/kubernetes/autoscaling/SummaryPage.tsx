// Autoscaling → Summary: what the clusters cost, how much of it is idle, and what scaling workloads
// (rightsized requests) and nodes (removing idle capacity) would save.
import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { Filter, X } from 'lucide-react';
import { Select } from '../../../components/ui/Dropdown';
import { money } from '../../cloudCost/costModel';
import { useOverview } from './AutoscalingLayout';
import { filterOverview, summarize } from './model';

const BANNER_KEY = 'oc.autoscaling.banner.dismissed';

function readDismissed(): boolean {
  try {
    return localStorage.getItem(BANNER_KEY) === '1';
  } catch {
    return false;
  }
}

export function SummaryPage() {
  const overview = useOverview();
  const [cluster, setCluster] = useState('');
  const [namespace, setNamespace] = useState('');
  const [dismissed, setDismissed] = useState(readDismissed);
  const view = useMemo(() => filterOverview(overview, cluster, namespace), [overview, cluster, namespace]);
  const s = useMemo(() => summarize(view.clusters, view.workloads, Boolean(namespace)), [view, namespace]);
  const namespaces = [...new Set(overview.workloads.filter((w) => !cluster || w.cluster === cluster).map((w) => w.namespace))].sort();
  const totalIdle = s.workloadIdleCost + s.clusterIdleCost;
  const total = s.usageCost + totalIdle;
  const savings = s.workloadSavings + s.clusterSavings;
  const pct = (v: number) => (total > 0 ? `${Math.round((v / total) * 100)}%` : '—');
  const width = (v: number) => `${total > 0 ? (v / total) * 100 : 0}%`;
  const top = [...view.clusters].sort((a, b) => b.projectedWorkloadSavings - a.projectedWorkloadSavings).slice(0, 5);
  const topMax = Math.max(...top.map((c) => c.projectedWorkloadSavings), 0);
  const dismiss = () => {
    setDismissed(true);
    try {
      localStorage.setItem(BANNER_KEY, '1');
    } catch {
      // Private mode: the banner comes back next time.
    }
  };
  return (
    <>
      <div className="ox-filterbar">
        <span className="ox-filter-title">
          <Filter size={15} /> Filter this page by
        </span>
        <span className="ox-filter">
          <span className="ox-filter-label">Cluster</span>
          <Select
            value={cluster}
            options={[{ value: '', label: 'all clusters' }, ...overview.clusters.map((c) => ({ value: c.cluster, label: c.cluster }))]}
            onChange={(v) => (setCluster(v), setNamespace(''))}
            triggerClassName="dd-trigger ox-filter-select"
          />
        </span>
        <span className="ox-filter">
          <span className="ox-filter-label">Namespace</span>
          <Select
            value={namespace}
            options={[{ value: '', label: 'all namespaces' }, ...namespaces.map((n) => ({ value: n, label: n }))]}
            onChange={setNamespace}
            triggerClassName="dd-trigger ox-filter-select"
          />
        </span>
      </div>
      <div className="as-body">
        {!dismissed && (
          <section className="as-banner">
            <div>
              <h2>Rightsize workloads and node capacity, then let the Cluster Agent scale them.</h2>
            </div>
            <ul>
              <li>Find workloads whose requests are far above what they use, and clusters with capacity no pod requests.</li>
              <li>Deploy DatadogPodAutoscalers through cluster profiles or per-workload manifests; ObserveCraft recommends their replicas.</li>
              <li>Follow the idle cost each autoscaler removes.</li>
            </ul>
            <button className="icon-plain as-banner-close" onClick={dismiss} title="Dismiss">
              <X size={18} />
            </button>
          </section>
        )}
        <section className="as-card">
          <h3 className="as-card-title">Optimization opportunities</h3>
          <div className="as-opportunities">
            <div className="as-save">
              <span className="as-save-label">You can save</span>
              <span className="muted">(potential savings per month)</span>
              <strong className="as-big">{total > 0 ? money(savings) : '—'}</strong>
              <span className="as-save-label">From a total idle cost of</span>
              <strong className="as-mid">{total > 0 ? money(totalIdle) : '—'}</strong>
            </div>
            <div className="as-costs">
              <div className="as-costbar" aria-label="Cost breakdown">
                <span className="as-seg as-seg-usage" style={{ width: width(s.usageCost) }} />
                <span className="as-seg as-seg-workload" style={{ width: width(s.workloadIdleCost - s.workloadSavings) }} />
                <span className="as-seg as-seg-workload as-striped" style={{ width: width(s.workloadSavings) }} />
                <span className="as-seg as-seg-cluster" style={{ width: width(s.clusterIdleCost - s.clusterSavings) }} />
                <span className="as-seg as-seg-cluster as-striped" style={{ width: width(s.clusterSavings) }} />
              </div>
              <span className="as-legend">
                <span className="as-swatch as-striped" /> Striped: potential savings
              </span>
              <table className="as-cost-table">
                <thead>
                  <tr>
                    <th>Cost category</th>
                    <th>Potential savings</th>
                    <th>Current cost</th>
                  </tr>
                </thead>
                <tbody>
                  {[
                    ['usage', 'Usage cost', null, s.usageCost],
                    ['workload', 'Workload idle cost', s.workloadSavings, s.workloadIdleCost],
                    ['cluster', 'Cluster idle cost', s.clusterSavings, s.clusterIdleCost],
                  ].map(([k, label, save, cost]) => (
                    <tr key={k as string}>
                      <td>
                        <span className={`as-swatch as-seg-${k}`} /> {label}
                      </td>
                      <td>{save == null || total === 0 ? '—' : money(save as number)}</td>
                      <td>{total === 0 ? '— (—)' : `${money(cost as number)} (${pct(cost as number)})`}</td>
                    </tr>
                  ))}
                  <tr className="as-total">
                    <td>Total</td>
                    <td>{total > 0 ? money(savings) : '—'}</td>
                    <td>{total > 0 ? money(total) : '—'}</td>
                  </tr>
                </tbody>
              </table>
            </div>
            <div className="as-top">
              <h4>Top clusters by potential workload savings</h4>
              {topMax > 0 ? (
                top.map((c) => (
                  <div key={c.cluster} className="as-top-row">
                    <span className="as-top-name">{c.cluster}</span>
                    <span className="as-top-bar">
                      <span style={{ width: `${(c.projectedWorkloadSavings / topMax) * 100}%` }} />
                    </span>
                    <span>{money(c.projectedWorkloadSavings)}</span>
                  </div>
                ))
              ) : (
                <p className="muted as-nodata">No Data</p>
              )}
            </div>
          </div>
          <div className="as-unscaled">
            <Unscaled title="Unscaled workloads" count={s.unscaledWorkloads} total={s.workloads} to="/kubernetes/autoscaling/setup" action="Set Up Workload Scaling" />
            <Unscaled title="Unscaled clusters" count={s.unscaledClusters} total={s.clusters} to="/kubernetes/autoscaling/setup?target=clusters" action="Set Up Cluster Scaling" />
          </div>
        </section>
      </div>
    </>
  );
}

function Unscaled({ title, count, total, to, action }: { title: string; count: number; total: number; to: string; action: string }) {
  return (
    <div className="as-unscaled-item">
      <h4>{title}</h4>
      <div className="as-ratio">
        <strong>{count}</strong>
        <span className="mono">/{total}</span>
        <span className="as-pct">{total ? Math.round((count / total) * 100) : 0}%</span>
      </div>
      <Link className="btn btn-primary" to={to}>
        {action}
      </Link>
    </div>
  );
}
