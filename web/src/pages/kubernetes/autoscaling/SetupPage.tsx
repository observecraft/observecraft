// Autoscaling → Setup: pick workloads (1), how the Cluster Agent scales them (2: a cluster profile
// through a label, or one DatadogPodAutoscaler each with ObserveCraft's recommender) and review the
// savings and the commands to run (3). Cluster scaling lists the clusters without a node autoscaler.
import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { ArrowRight } from 'lucide-react';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { formatBytes } from '../../../lib/format';
import { money } from '../../cloudCost/costModel';
import { useOverview } from './AutoscalingLayout';
import { NODE_AUTOSCALERS } from './ClusterScalingPage';
import { EmptyResults } from './EmptyResults';
import { DEFAULT_PROFILE, labelCommand, podAutoscalerManifest, profileTemplate, recommenderEndpoint, setResourcesCommand, type Dimensions } from './manifests';
import { workloadStatus } from './model';
import { WorkloadTable } from './WorkloadScalingPage';

const STEPS = ['Select Resources', 'Set Up Autoscaling', 'Review Savings'];
const defaultIntake = () => `${window.location.protocol}//${window.location.hostname}:4318`;

export function SetupPage() {
  const overview = useOverview();
  const [params, setParams] = useSearchParams();
  const target = params.get('target') === 'clusters' ? 'clusters' : 'workloads';
  const [step, setStep] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [readyOnly, setReadyOnly] = useState(true);
  const [query, setQuery] = useState('');
  const [method, setMethod] = useState<'profile' | 'recommender'>('profile');
  const [profile, setProfile] = useState(overview.profiles[0]?.name ?? 'datadog-optimize-balance');
  const [dimensions, setDimensions] = useState<Dimensions>('horizontal');
  const [intake, setIntake] = useState(defaultIntake);
  const candidates = overview.workloads.filter((w) => !w.autoscaler);
  const shown = candidates
    .filter((w) => (!readyOnly || workloadStatus(w) === 'ready') && `${w.cluster} ${w.namespace} ${w.name}`.toLowerCase().includes(query.trim().toLowerCase()))
    .sort((a, b) => b.projectedSavings - a.projectedSavings);
  const chosen = candidates.filter((w) => selected.has(w.uid));
  const savings = chosen.reduce((s, w) => s + w.projectedSavings, 0);
  const workloadSavings = candidates.reduce((s, w) => s + w.projectedSavings, 0);
  const clusterSavings = overview.clusters.reduce((s, c) => s + c.projectedClusterSavings, 0);
  const toggle = (uid: string, on: boolean) => setSelected((s) => (on ? new Set(s).add(uid) : new Set([...s].filter((x) => x !== uid))));
  const commands = useMemo(() => {
    if (method === 'profile') return chosen.map((w) => labelCommand(w, profile)).join('\n');
    const template = profileTemplate({ ...DEFAULT_PROFILE, dimensions });
    return chosen.map((w) => podAutoscalerManifest(w, template, recommenderEndpoint(intake))).join('---\n');
  }, [chosen, method, profile, dimensions, intake]);
  const rightsize = chosen
    .map((w) => setResourcesCommand(w, w.containers))
    .filter(Boolean)
    .join('\n');
  const pickTarget = (t: 'workloads' | 'clusters') => (setParams(t === 'workloads' ? {} : { target: t }), setStep(0));

  return (
    <>
      <div className="as-steps">
        <h2>Set up Kubernetes Autoscaling</h2>
        {STEPS.map((s, i) => (
          <span key={s} className={`as-step-pill${i === step && target === 'workloads' ? ' active' : ''}`}>
            {i > 0 && <ArrowRight size={16} className="as-step-arrow" />}
            <span className="as-step-num">{i + 1}</span> {s}
          </span>
        ))}
      </div>
      <div className="as-body">
        {step === 0 && (
          <div className="as-targets">
            <button className={`as-target${target === 'workloads' ? ' selected' : ''}`} onClick={() => pickTarget('workloads')}>
              <span>
                <strong>Workload Autoscaling</strong>
                <span className="muted">Adjust requests for CPU and memory, and replicas, dynamically</span>
              </span>
              <span className="as-target-money">
                <span>
                  <strong>{money(workloadSavings, { precise: true })}</strong>/mo
                </span>
                <span className="muted">Projected savings</span>
              </span>
            </button>
            <button className={`as-target${target === 'clusters' ? ' selected' : ''}`} onClick={() => pickTarget('clusters')}>
              <span>
                <strong>Cluster Autoscaling</strong>
                <span className="muted">Add or remove nodes based on idle capacity</span>
              </span>
              <span className="as-target-money">
                <span>
                  <strong>{money(clusterSavings, { precise: true })}</strong>/mo
                </span>
                <span className="muted">Projected savings</span>
              </span>
            </button>
          </div>
        )}

        {target === 'clusters' ? (
          <ClusterSetup />
        ) : step === 0 ? (
          <>
            <div className="as-filters">
              <label className="as-search">
                <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Filter your workloads" />
              </label>
              <label className="as-toggle">
                <input type="checkbox" checked={readyOnly} onChange={(e) => setReadyOnly(e.target.checked)} /> Ready workloads only
              </label>
            </div>
            <WorkloadTable workloads={shown} group="" empty="No workloads found" selected={selected} onSelect={toggle} />
            <div className="as-footer">
              <span>
                <strong>{chosen.length} workloads</strong> selected, projected savings of <strong>{money(savings, { precise: true })}</strong>/mo
              </span>
              <span className="as-actions">
                <button className="link-btn" disabled={!chosen.length} onClick={() => setSelected(new Set())}>
                  Deselect All
                </button>
                <button className="btn btn-primary" disabled={!chosen.length} onClick={() => setStep(1)}>
                  Select Autoscaling Profile
                </button>
              </span>
            </div>
          </>
        ) : step === 1 ? (
          <section className="as-card as-setup">
            <h3 className="as-card-title">How the Cluster Agent scales the {chosen.length} workloads</h3>
            <label className="as-method">
              <input type="radio" checked={method === 'profile'} onChange={() => setMethod('profile')} />
              <span>
                <strong>A cluster profile</strong>
                <span className="muted">Label the workloads; the Cluster Agent creates their autoscalers from the profile and scales replicas with its local recommendations.</span>
              </span>
            </label>
            {method === 'profile' && (
              <div className="as-method-body">
                <select value={profile} onChange={(e) => setProfile(e.target.value)}>
                  {overview.profiles.map((p) => (
                    <option key={p.name} value={p.name}>
                      {p.name} {p.clusters.length ? '' : '(not deployed yet)'}
                    </option>
                  ))}
                </select>
                <Link className="link-btn" to="/kubernetes/autoscaling/profiles/new">
                  Create a profile
                </Link>
              </div>
            )}
            <label className="as-method">
              <input type="radio" checked={method === 'recommender'} onChange={() => setMethod('recommender')} />
              <span>
                <strong>One autoscaler per workload, with ObserveCraft’s recommender</strong>
                <span className="muted">The Cluster Agent asks ObserveCraft every 30 seconds how many replicas each workload needs and applies the answer within its constraints.</span>
              </span>
            </label>
            {method === 'recommender' && (
              <div className="as-method-body">
                <select value={dimensions} onChange={(e) => setDimensions(e.target.value as Dimensions)}>
                  <option value="horizontal">Horizontal only</option>
                  <option value="both">Horizontal and vertical</option>
                </select>
                <label className="rt-field">
                  <span>Intake URL, as the cluster reaches it</span>
                  <input value={intake} onChange={(e) => setIntake(e.target.value)} />
                </label>
              </div>
            )}
            <div className="as-actions">
              <button className="btn" onClick={() => setStep(0)}>
                Back
              </button>
              <button className="btn btn-primary" onClick={() => setStep(2)}>
                Review Savings
              </button>
            </div>
          </section>
        ) : (
          <section className="as-card as-setup">
            <h3 className="as-card-title">Review savings</h3>
            <table className="settings-table as-table">
              <thead>
                <tr>
                  <th>Workload</th>
                  <th>Projected savings /mo</th>
                  <th>CPU idle</th>
                  <th>Mem unused</th>
                </tr>
              </thead>
              <tbody>
                {chosen.map((w) => (
                  <tr key={w.uid}>
                    <td>
                      {w.cluster} / {w.namespace} / <strong>{w.name}</strong>
                    </td>
                    <td className="as-money">{money(w.projectedSavings)}</td>
                    <td>{w.cpuIdle.toFixed(2)} cores</td>
                    <td>{formatBytes(w.memoryUnused)}</td>
                  </tr>
                ))}
                <tr className="as-total">
                  <td>Total</td>
                  <td className="as-money">{money(savings)}</td>
                  <td />
                  <td />
                </tr>
              </tbody>
            </table>
            <h4>{method === 'profile' ? '1. Label the workloads' : '1. Apply the autoscalers'}</h4>
            {method === 'recommender' && (
              <p className="muted">
                Replace <code>&lt;INGEST_API_KEY&gt;</code> with an ingest key. The Cluster Agent needs <code>DD_AUTOSCALING_WORKLOAD_EXTERNAL_RECOMMENDER_ENABLED=true</code> (Integrations →
                Kubernetes → Install sets it).
              </p>
            )}
            <CodeBlock code={commands} filename={method === 'profile' ? 'label.sh' : 'autoscalers.yaml'} />
            {rightsize && (
              <>
                <h4>2. Rightsize their requests</h4>
                <p className="muted">The vertical values the savings assume; the Cluster Agent only receives them through Datadog’s Remote Configuration.</p>
                <CodeBlock code={rightsize} filename="rightsize.sh" />
              </>
            )}
            <div className="as-actions">
              <button className="btn" onClick={() => setStep(1)}>
                Back
              </button>
              <Link className="btn btn-primary" to="/kubernetes/autoscaling/workloads?view=optimized">
                Done
              </Link>
            </div>
          </section>
        )}
      </div>
    </>
  );
}

function ClusterSetup() {
  const { clusters } = useOverview();
  const unscaled = clusters.filter((c) => c.autoscaling === 'none').sort((a, b) => b.projectedClusterSavings - a.projectedClusterSavings);
  return (
    <section className="as-card as-setup">
      <h3 className="as-card-title">Clusters without a node autoscaler</h3>
      <p className="muted">
        Nodes are added and removed by a node autoscaler running in the cluster: Karpenter (NodePools) or the Kubernetes Cluster Autoscaler, as your platform provides. Once it runs, ObserveCraft
        detects it and the cluster leaves this list. Savings count the idle capacity beyond one node of headroom.
      </p>
      <table className="settings-table as-table">
        <thead>
          <tr>
            <th>Cluster</th>
            <th>Autoscaling</th>
            <th>Nodes</th>
            <th>CPU requested</th>
            <th>Projected savings /mo</th>
          </tr>
        </thead>
        <tbody>
          {unscaled.map((c) => (
            <tr key={c.cluster}>
              <td>{c.cluster}</td>
              <td>{NODE_AUTOSCALERS[c.autoscaling]}</td>
              <td>{c.nodes}</td>
              <td>{c.cpuCapacity ? `${Math.round((c.cpuRequests / c.cpuCapacity) * 100)}% of ${c.cpuCapacity.toFixed(1)} cores` : '—'}</td>
              <td className="as-money">{money(c.projectedClusterSavings)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {unscaled.length === 0 && <EmptyResults title="Every cluster has a node autoscaler" />}
    </section>
  );
}
