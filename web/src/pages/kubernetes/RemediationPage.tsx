// Kubernetes → Remediation: Deployments, DaemonSets and StatefulSets whose pods have a common problem,
// where they are, and what to check first.
import { Fragment, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { ArrowRight, ChevronDown, ChevronRight, HeartPulse, PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { listWorkloadProblems, type WorkloadProblem } from '../../api/kubernetes/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { applyFacets, facetValues } from './autoscaling/model';
import { Facets, type FacetDef } from './Facets';
import { LiveClock, LiveProvider, OrchestrationHeader, useLive } from './OrchestrationHeader';
import '../infrastructure/infrastructure.css';
import '../settings/settings.css';
import './kubernetes.css';
import './autoscaling/autoscaling.css';

const FACETS: FacetDef<WorkloadProblem>[] = [
  { key: 'cluster', title: 'Cluster', value: (p) => p.cluster },
  { key: 'namespace', title: 'Namespace', value: (p) => p.namespace },
  { key: 'issue', title: 'Issue type', value: (p) => p.issueLabel },
  { key: 'kind', title: 'Workload type', value: (p) => p.kind },
  { key: 'team', title: 'Team', value: (p) => p.team },
  { key: 'service', title: 'Service', value: (p) => p.service },
];
const KEYS = Object.fromEntries(FACETS.map((f) => [f.key, f.value]));

export function RemediationPage() {
  return (
    <LiveProvider>
      <Remediation />
    </LiveProvider>
  );
}

function Remediation() {
  const { interval } = useLive();
  const problems = useQuery({ queryKey: ['kubernetes', 'remediation'], queryFn: listWorkloadProblems, refetchInterval: interval });
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<Record<string, string[]>>({});
  const [controls, setControls] = useState(true);
  const [open, setOpen] = useState<string | null>(null);
  const all = problems.data ?? [];
  const q = query.trim().toLowerCase();
  const shown = useMemo(
    () => applyFacets(all, KEYS, selected).filter((p) => !q || [p.cluster, p.namespace, p.name, p.issueLabel, p.service ?? '', p.team ?? ''].some((v) => v.toLowerCase().includes(q))),
    [all, selected, q],
  );
  const rowKey = (p: WorkloadProblem) => `${p.cluster}/${p.namespace}/${p.kind}/${p.name}/${p.issue}`;
  return (
    <div className="hosts-page ox-page as-page">
      <OrchestrationHeader product="kubernetes" active="Remediation" right={<LiveClock updatedAt={problems.dataUpdatedAt} />} />
      <div className="as-body">
        <label className="as-search">
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Filter by tag" />
        </label>
        <div className={`as-explorer${controls ? '' : ' no-controls'}`}>
          {controls && <Facets items={all} facets={FACETS} selected={selected} onChange={setSelected} />}
          <div className="as-explorer-main">
            <button className="link-btn as-hide" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            {problems.isPending ? (
              <Loading label="Loading workloads…" />
            ) : problems.isError ? (
              <ErrorState error={problems.error} onRetry={() => problems.refetch()} />
            ) : (
              <>
                <div className="as-charts">
                  <TopList title="Top Clusters with Workload Problems" items={shown} value={(p) => p.cluster} />
                  <TopList title="Top Services with Workload Problems" items={shown} value={(p) => p.service} />
                  <TopList title="Workload Problems by Issue Type" items={shown} value={(p) => p.issueLabel} />
                </div>
                <p className="as-count">{shown.length ? `${shown.length} problematic workloads` : 'No problematic workloads found'}</p>
                <div className="as-table-card">
                  <table className="settings-table as-table">
                    <thead>
                      <tr>
                        <th>Type</th>
                        <th>Workload</th>
                        <th>Pods count</th>
                        <th>% Pods affected</th>
                        <th>Started</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {shown.map((p) => (
                        <Fragment key={rowKey(p)}>
                          <tr className="as-click" onClick={() => setOpen(open === rowKey(p) ? null : rowKey(p))}>
                            <td>
                              <span className="as-pill as-pill-off">{p.issueLabel}</span>
                            </td>
                            <td>
                              <div className="as-workload">
                                <strong>{p.name}</strong>
                                <span className="muted">
                                  {p.kind} · {p.cluster} / {p.namespace}
                                </span>
                              </div>
                            </td>
                            <td>
                              {p.podsAffected} / {p.podsTotal}
                            </td>
                            <td>{p.podsTotal ? Math.round((p.podsAffected / p.podsTotal) * 100) : 0}%</td>
                            <td>{p.started ? timeAgo(p.started) : '—'}</td>
                            <td>{open === rowKey(p) ? <ChevronDown size={16} /> : <ChevronRight size={16} />}</td>
                          </tr>
                          {open === rowKey(p) && (
                            <tr className="as-detail-row">
                              <td colSpan={6}>
                                <div className="as-detail">
                                  <strong>How to fix it</strong>
                                  <p>{p.remediation}</p>
                                  <span className="muted">Affected pods: {p.pods.join(', ')}</span>
                                  <Link to={`/orchestration/explorer?resource=pods&query=${encodeURIComponent(`kube_namespace:${p.namespace} kube_cluster_name:${p.cluster} ${p.name}`)}`}>
                                    Open the pods in the Kubernetes Explorer <ArrowRight size={14} />
                                  </Link>
                                </div>
                              </td>
                            </tr>
                          )}
                        </Fragment>
                      ))}
                    </tbody>
                  </table>
                  {shown.length === 0 && (
                    <div className="as-empty">
                      <HeartPulse size={64} strokeWidth={1.5} />
                      <h3>Nothing to fix right now!</h3>
                      <p>Remediations for common problems of your Deployments, DaemonSets and StatefulSets (crash loops, out-of-memory kills, image pulls, scheduling, readiness) show up here.</p>
                      <Link className="btn btn-primary" to="/orchestration/explorer">
                        Kubernetes Explorer <ArrowRight size={15} />
                      </Link>
                    </div>
                  )}
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function TopList({ title, items, value }: { title: string; items: WorkloadProblem[]; value: (p: WorkloadProblem) => string | null | undefined }) {
  const top = facetValues(items, value).slice(0, 5);
  const max = Math.max(...top.map((t) => t.count), 1);
  return (
    <section className="as-chart">
      <h4>{title}</h4>
      {top.length ? (
        top.map((t) => (
          <div key={t.value} className="as-top-row">
            <span className="as-top-name">{t.value}</span>
            <span className="as-top-bar">
              <span style={{ width: `${(t.count / max) * 100}%` }} />
            </span>
            <span>{t.count}</span>
          </div>
        ))
      ) : (
        <p className="muted as-nodata">No Data</p>
      )}
    </section>
  );
}
