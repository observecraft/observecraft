// Autoscaling → Workload Scaling: rightsizing recommendations of unscaled workloads, the workloads
// autoscalers already optimize, autoscalers left misconfigured, and cluster profiles.
import { Fragment, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { ChevronDown, ChevronRight, Pencil, Plus, Search, Trash2 } from 'lucide-react';
import type { KubeAutoscaler, StoredRecommendation, WorkloadRecommendation } from '../../../api/kubernetes/client';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { Select } from '../../../components/ui/Dropdown';
import { formatBytes, timeAgo } from '../../../lib/format';
import { money } from '../../cloudCost/costModel';
import { useOverview } from './AutoscalingLayout';
import { EmptyResults } from './EmptyResults';
import { cores, mebibytes, setResourcesCommand, uninstallCommands } from './manifests';
import { workloadStatus } from './model';
import { useAutoscalingMutations } from './queries';

const VIEWS = [
  { id: 'recommendations', label: 'Scaling Recommendations' },
  { id: 'optimized', label: 'Optimized Workloads' },
  { id: 'misconfigured', label: 'Misconfigured Autoscalers' },
  { id: 'profiles', label: 'Profiles' },
] as const;
type View = (typeof VIEWS)[number]['id'];

const GROUPS = [
  { value: '', label: 'Group your workloads' },
  { value: 'cluster', label: 'Cluster' },
  { value: 'namespace', label: 'Namespace' },
  { value: 'service', label: 'Service' },
] as const;
type Group = (typeof GROUPS)[number]['value'];

const matches = (w: { cluster: string; namespace: string; name: string }, q: string) => !q || `${w.cluster} ${w.namespace} ${w.name}`.toLowerCase().includes(q);
const groupOf = (w: WorkloadRecommendation, g: Group) => (g === 'cluster' ? w.cluster : g === 'namespace' ? `${w.cluster} / ${w.namespace}` : g === 'service' ? (w.service ?? 'no service') : '');
const pct = (part: number, whole: number) => (whole > 0 ? `${Math.round((part / whole) * 100)}%` : '—');

export function WorkloadScalingPage() {
  const [params, setParams] = useSearchParams();
  const view: View = VIEWS.find((v) => v.id === params.get('view'))?.id ?? 'recommendations';
  const [query, setQuery] = useState('');
  const q = query.trim().toLowerCase();
  return (
    <div className="as-workloads">
      <aside className="as-sidenav">
        {VIEWS.map((v) => (
          <button key={v.id} className={view === v.id ? 'active' : ''} onClick={() => (setParams(v.id === 'recommendations' ? {} : { view: v.id }), setQuery(''))}>
            {v.label}
          </button>
        ))}
      </aside>
      <div className="as-view">
        <label className="as-search">
          <Search size={16} />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={view === 'misconfigured' ? 'Filter your autoscalers' : view === 'profiles' ? 'Filter your cluster profiles' : 'Filter your workloads'}
          />
        </label>
        {view === 'recommendations' && <Recommendations q={q} />}
        {view === 'optimized' && <Optimized q={q} />}
        {view === 'misconfigured' && <Misconfigured q={q} />}
        {view === 'profiles' && <Profiles q={q} />}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- recommendations

function Recommendations({ q }: { q: string }) {
  const { workloads } = useOverview();
  const [group, setGroup] = useState<Group>('');
  const [agent, setAgent] = useState<'any' | 'installed' | 'missing'>('any');
  const [readiness, setReadiness] = useState<'any' | 'ready' | 'not_ready'>('any');
  const unscaled = workloads.filter((w) => !w.autoscaler);
  const shown = unscaled
    .filter((w) => matches(w, q))
    .filter((w) => agent === 'any' || (agent === 'installed') === w.agentSetup)
    .filter((w) => readiness === 'any' || workloadStatus(w) === readiness)
    .sort((a, b) => b.projectedSavings - a.projectedSavings);
  return (
    <>
      <div className="as-filters">
        <Select label="Group By" value={group} options={[...GROUPS]} onChange={setGroup} />
        <Select
          label="Agent Setup"
          value={agent}
          options={[
            { value: 'any', label: 'Any' },
            { value: 'installed', label: 'Workload autoscaling installed' },
            { value: 'missing', label: 'Not installed' },
          ]}
          onChange={setAgent}
        />
        <Select
          label="Workload Readiness"
          value={readiness}
          options={[
            { value: 'any', label: 'Any' },
            { value: 'ready', label: 'Ready' },
            { value: 'not_ready', label: 'Not ready' },
          ]}
          onChange={setReadiness}
        />
      </div>
      <div className="as-view-head">
        <div>
          <h2>Scaling Recommendations</h2>
          <p>Optimize the following workloads to have the greatest impact on savings.</p>
        </div>
        <Link className="btn btn-primary" to="/kubernetes/autoscaling/setup">
          Set Up Workload Scaling
        </Link>
      </div>
      <div className="as-kpis">
        <div>
          <span className="as-kpi-label">Projected monthly savings</span>
          <strong className="as-big">
            {money(
              unscaled.reduce((s, w) => s + w.projectedSavings, 0),
              { precise: true },
            )}
          </strong>
          <span className="muted">Based on the last 24 hours of CPU and memory usage, over a month.</span>
        </div>
        <div>
          <span className="as-kpi-label">Unscaled workloads</span>
          <span className="as-ratio">
            <strong>{unscaled.length}</strong>
            <span className="mono">/{workloads.length}</span>
          </span>
        </div>
      </div>
      <WorkloadTable workloads={shown} group={group} empty="No workloads found" />
    </>
  );
}

export function WorkloadTable({
  workloads,
  group,
  empty,
  selected,
  onSelect,
}: {
  workloads: WorkloadRecommendation[];
  group: Group;
  empty: string;
  selected?: Set<string>;
  onSelect?: (uid: string, on: boolean) => void;
}) {
  const [open, setOpen] = useState<string | null>(null);
  const groups = useMemo(() => {
    const m = new Map<string, WorkloadRecommendation[]>();
    for (const w of workloads) m.set(groupOf(w, group), [...(m.get(groupOf(w, group)) ?? []), w]);
    return [...m.entries()];
  }, [workloads, group]);
  const columns = onSelect ? 7 : 6;
  return (
    <div className="as-table-card">
      <div className="as-table-head">
        <strong>{workloads.length}</strong> workloads
      </div>
      <table className="settings-table as-table">
        <thead>
          <tr>
            {onSelect && (
              <th className="as-check">
                <input
                  type="checkbox"
                  aria-label="Select all"
                  checked={workloads.length > 0 && workloads.every((w) => selected?.has(w.uid))}
                  onChange={(e) => workloads.forEach((w) => onSelect(w.uid, e.target.checked))}
                />
              </th>
            )}
            <th>Projected savings</th>
            <th>Workload</th>
            <th>CPU idle</th>
            <th>Mem unused</th>
            <th>Workload readiness</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {groups.map(([g, ws]) => (
            <Fragment key={g}>
              {group && (
                <tr className="as-group-row">
                  <td colSpan={columns}>
                    {g} <span className="muted">({ws.length})</span>
                  </td>
                </tr>
              )}
              {ws.map((w) => (
                <Fragment key={w.uid}>
                  <tr className="as-click" onClick={() => setOpen(open === w.uid ? null : w.uid)}>
                    {onSelect && (
                      <td className="as-check" onClick={(e) => e.stopPropagation()}>
                        <input
                          type="checkbox"
                          aria-label={`Select ${w.name}`}
                          checked={selected?.has(w.uid) ?? false}
                          onChange={(e) => onSelect(w.uid, e.target.checked)}
                          disabled={workloadStatus(w) !== 'ready'}
                        />
                      </td>
                    )}
                    <td className="as-money">{money(w.projectedSavings)}</td>
                    <td>
                      <div className="as-workload">
                        <strong>{w.name}</strong>
                        <span className="muted">
                          {w.kind} · {w.cluster} / {w.namespace} · {w.replicas} replicas
                        </span>
                      </div>
                    </td>
                    <td>{w.cpuIdle.toFixed(2)} cores</td>
                    <td>{formatBytes(w.memoryUnused)}</td>
                    <td>
                      <Readiness w={w} />
                    </td>
                    <td>{open === w.uid ? <ChevronDown size={16} /> : <ChevronRight size={16} />}</td>
                  </tr>
                  {open === w.uid && (
                    <tr className="as-detail-row">
                      <td colSpan={columns}>
                        <WorkloadDetail w={w} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              ))}
            </Fragment>
          ))}
        </tbody>
      </table>
      {workloads.length === 0 && <EmptyResults title={empty} />}
    </div>
  );
}

function Readiness({ w }: { w: WorkloadRecommendation }) {
  const status = workloadStatus(w);
  if (status === 'ready') return <span className="as-pill as-pill-on">Ready</span>;
  return (
    <span className={`as-pill ${status === 'scaled' ? 'as-pill-info' : 'as-pill-off'}`} title={w.notReady.join('\n')}>
      {status === 'scaled' ? 'Autoscaled' : 'Not ready'}
      {status === 'not_ready' && <span className="as-reason"> · {w.notReady[0]}</span>}
    </span>
  );
}

function WorkloadDetail({ w }: { w: WorkloadRecommendation }) {
  const command = setResourcesCommand(w, w.containers);
  return (
    <div className="as-detail">
      <table className="as-mini-table">
        <thead>
          <tr>
            <th>Container</th>
            <th>CPU request</th>
            <th>CPU p95 / avg</th>
            <th>Recommended CPU</th>
            <th>Memory request</th>
            <th>Memory max / avg</th>
            <th>Recommended memory</th>
          </tr>
        </thead>
        <tbody>
          {w.containers.map((c) => (
            <tr key={c.name}>
              <td className="mono">{c.name}</td>
              <td>{c.cpuRequest != null ? cores(c.cpuRequest) : '—'}</td>
              <td>{c.cpuP95 != null ? `${cores(c.cpuP95)} / ${cores(c.cpuAvg ?? 0)}` : '—'}</td>
              <td>
                <strong>{c.recommendedCpu != null ? cores(c.recommendedCpu) : '—'}</strong>
              </td>
              <td>{c.memoryRequest != null ? mebibytes(c.memoryRequest) : '—'}</td>
              <td>{c.memoryMax != null ? `${mebibytes(c.memoryMax)} / ${mebibytes(c.memoryAvg ?? 0)}` : '—'}</td>
              <td>
                <strong>{c.recommendedMemory != null ? mebibytes(c.recommendedMemory) : '—'}</strong>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {w.notReady.length > 0 && (
        <ul className="as-reasons">
          {w.notReady.map((r) => (
            <li key={r}>{r}</li>
          ))}
        </ul>
      )}
      {command && (
        <>
          <p className="muted">Apply the recommended requests once (a vertical autoscaler keeps them current):</p>
          <CodeBlock code={command} filename="rightsize.sh" />
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- optimized

function lastAnswer(a: KubeAutoscaler, answers: StoredRecommendation[]) {
  return answers.find((r) => r.cluster === a.cluster && r.namespace === a.namespace && r.kind === a.targetKind && r.name === a.targetName);
}

function Optimized({ q }: { q: string }) {
  const { workloads, autoscalers, recommendations } = useOverview();
  const active = autoscalers.filter((a) => a.targetExists && matches(a, q));
  const workloadOf = (a: KubeAutoscaler) => workloads.find((w) => w.cluster === a.cluster && w.namespace === a.namespace && w.kind === a.targetKind && w.name === a.targetName);
  const scaled = workloads.filter((w) => w.autoscaler);
  const idle = scaled.reduce((s, w) => s + w.idleCost, 0);
  const requested = scaled.reduce((s, w) => s + w.requestsCost, 0);
  return (
    <>
      <div className="as-view-head">
        <div>
          <h2>Optimized Workloads</h2>
          <p>Monitor the autoscalers running on your workloads.</p>
        </div>
      </div>
      <div className="as-kpis">
        <div>
          <span className="as-kpi-label">Idle cost of autoscaled workloads</span>
          <strong className="as-big">{scaled.length ? money(idle) : '—'}</strong>
          <span className="muted">Per month, at the last 24 hours of usage.</span>
        </div>
        <div>
          <span className="as-kpi-label">Total idle cost %</span>
          <strong className="as-big">{scaled.length ? pct(idle, requested) : '—'}</strong>
        </div>
        <div>
          <span className="as-kpi-label">Optimized workloads</span>
          <span className="as-ratio">
            <strong>{scaled.length}</strong>
            <span className="mono">/{workloads.length}</span>
          </span>
        </div>
      </div>
      <div className="as-table-card">
        <div className="as-table-head">
          <strong>{active.length}</strong> workloads
        </div>
        <table className="settings-table as-table">
          <thead>
            <tr>
              <th>Workload</th>
              <th>Autoscaling</th>
              <th>Dimension</th>
              <th>Pod statuses</th>
              <th>Replicas</th>
              <th>Recommender</th>
              <th>Idle cost /mo</th>
              <th>Idle cost %</th>
            </tr>
          </thead>
          <tbody>
            {active.map((a) => {
              const w = workloadOf(a);
              const answer = lastAnswer(a, recommendations);
              return (
                <tr key={a.uid}>
                  <td>
                    <div className="as-workload">
                      <strong>{a.targetName}</strong>
                      <span className="muted">
                        {a.targetKind} · {a.cluster} / {a.namespace}
                      </span>
                    </div>
                  </td>
                  <td>
                    {a.kind === 'DatadogPodAutoscaler' ? 'DPA' : 'HPA'}
                    {a.profile && <span className="muted"> · {a.profile}</span>}
                  </td>
                  <td>{[a.horizontal && 'Horizontal', a.vertical && 'Vertical'].filter(Boolean).join(' + ') || '—'}</td>
                  <td>
                    <span className="as-pods">
                      <span className="as-pod-ok">{a.podsRunning} running</span>
                      {a.podsPending > 0 && <span className="as-pod-warn">{a.podsPending} pending</span>}
                      {a.podsFailing > 0 && <span className="as-pod-err">{a.podsFailing} failing</span>}
                    </span>
                  </td>
                  <td>
                    {a.currentReplicas ?? '—'}
                    {a.minReplicas != null || a.maxReplicas != null ? (
                      <span className="muted">
                        {' '}
                        ({a.minReplicas ?? 1}–{a.maxReplicas ?? '∞'})
                      </span>
                    ) : null}
                  </td>
                  <td>
                    {answer ? (
                      <span title={answer.error ?? answer.reason}>
                        {answer.error ? <span className="as-pod-err">{answer.error}</span> : `${answer.targetReplicas} replicas`} <span className="muted">· {timeAgo(answer.updatedAt)}</span>
                      </span>
                    ) : a.recommender ? (
                      <span className="muted">Waiting for the first call</span>
                    ) : (
                      <span className="muted">Cluster Agent</span>
                    )}
                  </td>
                  <td>{w ? money(w.idleCost) : '—'}</td>
                  <td>{w ? pct(w.idleCost, w.requestsCost) : '—'}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {active.length === 0 && <EmptyResults title="No active autoscalers found" />}
      </div>
    </>
  );
}

// ---------------------------------------------------------------- misconfigured

function Misconfigured({ q }: { q: string }) {
  const { autoscalers } = useOverview();
  const broken = autoscalers.filter((a) => a.problems.length && matches(a, q));
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [showCommands, setShowCommands] = useState(false);
  const toggle = (uid: string, on: boolean) => setSelected((s) => (on ? new Set(s).add(uid) : new Set([...s].filter((x) => x !== uid))));
  const chosen = broken.filter((a) => selected.has(a.uid));
  return (
    <>
      <div className="as-view-head">
        <div>
          <h2>Misconfigured Autoscalers</h2>
          <p>Autoscalers whose target workload no longer exists or that report errors. Uninstall them to prevent unintended scaling.</p>
        </div>
      </div>
      <div className="as-kpis">
        <div>
          <span className="as-kpi-label">Total autoscalers</span>
          <strong className="as-big">{broken.length}</strong>
        </div>
      </div>
      <div className="as-table-card">
        <div className="as-table-head">
          <strong>{broken.length}</strong> autoscalers
          <button className="btn btn-sm" disabled={!chosen.length} onClick={() => setShowCommands(true)}>
            <Trash2 size={14} /> Uninstall Selected
          </button>
        </div>
        {showCommands && chosen.length > 0 && (
          <div className="as-commands">
            <p className="muted">ObserveCraft doesn’t change your clusters. Run these to remove the selected autoscalers:</p>
            <CodeBlock code={uninstallCommands(chosen).join('\n')} filename="uninstall.sh" />
          </div>
        )}
        <table className="settings-table as-table">
          <thead>
            <tr>
              <th className="as-check">
                <input
                  type="checkbox"
                  aria-label="Select all"
                  checked={broken.length > 0 && broken.every((a) => selected.has(a.uid))}
                  onChange={(e) => broken.forEach((a) => toggle(a.uid, e.target.checked))}
                />
              </th>
              <th>Cluster</th>
              <th>Namespace</th>
              <th>Autoscaler name</th>
              <th>Owner</th>
              <th>Created</th>
              <th>Configuration summary</th>
              <th>Uninstall autoscaler</th>
            </tr>
          </thead>
          <tbody>
            {broken.map((a) => (
              <tr key={a.uid}>
                <td className="as-check">
                  <input type="checkbox" aria-label={`Select ${a.name}`} checked={selected.has(a.uid)} onChange={(e) => toggle(a.uid, e.target.checked)} />
                </td>
                <td>{a.cluster}</td>
                <td>{a.namespace}</td>
                <td>
                  {a.name} <span className="muted">({a.kind === 'DatadogPodAutoscaler' ? 'DPA' : 'HPA'})</span>
                </td>
                <td>{a.profile ?? a.owner ?? '—'}</td>
                <td>{a.createdAt ? timeAgo(a.createdAt) : '—'}</td>
                <td>{a.problems.join('; ')}</td>
                <td>
                  <code className="as-inline-code">{uninstallCommands([a])[0]}</code>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {broken.length === 0 && <EmptyResults title="No matching results found" />}
      </div>
    </>
  );
}

// ---------------------------------------------------------------- profiles

const SOURCES = { builtin: 'Built-in', cluster: 'In cluster', saved: 'Saved in ObserveCraft' } as const;

function Profiles({ q }: { q: string }) {
  const { profiles } = useOverview();
  const { remove } = useAutoscalingMutations();
  const [name, setName] = useState('');
  const [cluster, setCluster] = useState('');
  const clusters = [...new Set(profiles.flatMap((p) => p.clusters))].sort();
  const shown = profiles.filter((p) => (!q || p.name.toLowerCase().includes(q)) && (!name || p.name === name) && (!cluster || p.clusters.includes(cluster)));
  return (
    <>
      <div className="as-filters">
        <Select label="Profile Name" value={name} options={[{ value: '', label: 'Any profile' }, ...profiles.map((p) => ({ value: p.name, label: p.name }))]} onChange={setName} />
        <Select label="Cluster Name" value={cluster} options={[{ value: '', label: 'Any cluster' }, ...clusters.map((c) => ({ value: c, label: c }))]} onChange={setCluster} />
      </div>
      <div className="as-view-head">
        <div>
          <h2>Profiles</h2>
          <p>Cluster-scoped autoscaling templates. The Cluster Agent creates an autoscaler for each workload that references a profile through its label.</p>
        </div>
        <Link className="btn btn-primary" to="/kubernetes/autoscaling/profiles/new">
          <Plus size={15} /> Create Profile
        </Link>
      </div>
      <div className="as-table-card">
        <div className="as-table-head">
          <strong>{shown.length}</strong> profiles
        </div>
        <table className="settings-table as-table">
          <thead>
            <tr>
              <th>Profile</th>
              <th>Source</th>
              <th>Deployed in</th>
              <th>Workloads</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {shown.map((p) => (
              <tr key={p.name}>
                <td className="mono">{p.name}</td>
                <td>{SOURCES[p.source]}</td>
                <td>{p.clusters.length ? p.clusters.join(', ') : <span className="muted">Not deployed yet</span>}</td>
                <td>{p.workloads}</td>
                <td className="as-row-actions">
                  {p.id ? (
                    <>
                      <Link className="icon-plain" to={`/kubernetes/autoscaling/profiles/${p.id}`} title="Edit">
                        <Pencil size={15} />
                      </Link>
                      <button className="icon-plain" title="Delete from ObserveCraft" disabled={remove.isPending} onClick={() => remove.mutate(p.id!)}>
                        <Trash2 size={15} />
                      </button>
                    </>
                  ) : (
                    <Link className="link-btn" to={`/kubernetes/autoscaling/profiles/new?from=${encodeURIComponent(p.name)}`}>
                      Copy
                    </Link>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {shown.length === 0 && <EmptyResults title="No cluster profile groups found" />}
      </div>
    </>
  );
}
