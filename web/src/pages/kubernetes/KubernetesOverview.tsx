import { useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { Box, ChevronDown, ChevronUp, Filter, type LucideIcon } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { searchKubernetes, type KubeKind } from '../../api/kubernetes/client';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { KubernetesIcon } from '../../components/brand/KubernetesIcon';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { compact } from '../../lib/format';
import { LiveClock, LiveProvider, OrchestrationHeader, useLive } from './OrchestrationHeader';
import '../infrastructure/infrastructure.css';
import './kubernetes.css';

// Kinds show the Kubernetes project's icons (KubernetesIcon); containers, which have none, a container.
const CARDS: { kind: KubeKind; label: string; icon?: LucideIcon }[] = [
  { kind: 'clusters', label: 'Clusters' },
  { kind: 'namespaces', label: 'Namespaces' },
  { kind: 'nodes', label: 'Nodes' },
  { kind: 'deployments', label: 'Deployments' },
  { kind: 'pods', label: 'Pods' },
  { kind: 'containers', label: 'Containers', icon: Box },
  { kind: 'replicasets', label: 'Replica Sets' },
  { kind: 'services', label: 'Services' },
];

const MORE: { kind: KubeKind; label: string; icon?: LucideIcon }[] = [
  { kind: 'daemonsets', label: 'Daemon Sets' },
  { kind: 'statefulsets', label: 'Stateful Sets' },
  { kind: 'jobs', label: 'Jobs' },
  { kind: 'cronjobs', label: 'Cron Jobs' },
  { kind: 'ingresses', label: 'Ingresses' },
  { kind: 'persistentvolumeclaims', label: 'Persistent Volume Claims' },
  { kind: 'persistentvolumes', label: 'Persistent Volumes' },
  { kind: 'horizontalpodautoscalers', label: 'Horizontal Pod Autoscalers' },
];

/** Infrastructure → Kubernetes Overview. */
export function KubernetesOverview() {
  return (
    <LiveProvider>
      <OverviewBody />
    </LiveProvider>
  );
}

function OverviewBody() {
  const navigate = useNavigate();
  const { interval } = useLive();
  const [cluster, setCluster] = useState('');
  const [namespace, setNamespace] = useState('');
  const [more, setMore] = useState(false);
  const query = [cluster && `kube_cluster_name:${cluster}`, namespace && `kube_namespace:${namespace}`].filter(Boolean).join(' ');

  // Namespaces (with their cluster facet) feed both pickers and, filtered, every count.
  const ns = useQuery({
    queryKey: ['kubernetes', 'overview', 'namespaces', cluster],
    queryFn: () => searchKubernetes({ kind: 'namespaces', query: cluster ? `kube_cluster_name:${cluster}` : '', limit: 1000 }),
    refetchInterval: interval,
  });
  const counts = useQuery({
    queryKey: ['kubernetes', 'overview', 'counts', query],
    queryFn: () => searchKubernetes({ kind: 'deployments', query, limit: 1000 }),
    placeholderData: keepPreviousData,
    refetchInterval: interval,
  });
  const clusters = useQuery({ queryKey: ['kubernetes', 'overview', 'clusters'], queryFn: () => searchKubernetes({ kind: 'clusters', query: '', limit: 1000 }), refetchInterval: interval });
  const hpas = useQuery({ queryKey: ['kubernetes', 'overview', 'hpas', query], queryFn: () => searchKubernetes({ kind: 'horizontalpodautoscalers', query, limit: 1000 }), refetchInterval: interval });

  const count = useMemo(() => new Map((counts.data?.kinds ?? []).map((k) => [k.kind, k.count])), [counts.data]);
  const open = (kind: KubeKind) => navigate(`/orchestration/explorer?resource=${kind}${query ? `&q=${encodeURIComponent(query)}` : ''}`);

  // Workloads that set CPU requests can be autoscaled; those already targeted by an HPA are scaled.
  const workloads = counts.data?.items ?? [];
  const scaled = new Set((hpas.data?.items ?? []).map((h) => `${h.cluster}/${h.namespace}/${h.target?.split('/')[1] ?? ''}`));
  const ready = workloads.filter((w) => w.cpu?.requests != null && !scaled.has(`${w.cluster}/${w.namespace}/${w.name}`)).length;
  // The same workloads as the percentage (Deployments), so "63 %" is a share of this total.
  const totalWorkloads = count.get('deployments') ?? workloads.length;

  return (
    <div className="hosts-page ox-page">
      <OrchestrationHeader product="kubernetes" />
      <div className="ox-filterbar">
        <span className="ox-filter-title">
          <Filter size={15} /> Filter this page by
        </span>
        <span className="ox-filter">
          <span className="ox-filter-label">Cluster</span>
          <Select
            value={cluster}
            options={[{ value: '', label: 'all clusters' }, ...(clusters.data?.items ?? []).map((c) => ({ value: c.name, label: c.name }))]}
            onChange={(v) => (setCluster(v), setNamespace(''))}
            triggerClassName="dd-trigger ox-filter-select"
          />
        </span>
        <span className="ox-filter">
          <span className="ox-filter-label">Namespace</span>
          <Select
            value={namespace}
            options={[{ value: '', label: 'all namespaces' }, ...[...new Set((ns.data?.items ?? []).map((n) => n.name))].sort().map((n) => ({ value: n, label: n }))]}
            onChange={setNamespace}
            triggerClassName="dd-trigger ox-filter-select"
          />
        </span>
        <span className="ox-header-right">
          <LiveClock updatedAt={counts.dataUpdatedAt} />
        </span>
      </div>

      <div className="ox-overview">
        {counts.isPending ? (
          <Loading />
        ) : counts.isError ? (
          <ErrorState error={counts.error} onRetry={() => counts.refetch()} />
        ) : (
          <>
            <section className="ox-hero">
              <div className="ox-hero-text">
                <BrandLogo name="kubernetes" size={44} title="Kubernetes" />
                <h1>
                  Check the state of all of your <span className="ox-accent">Kubernetes</span> resources
                </h1>
              </div>
              <div className="ox-cards">
                <div className="ox-card-grid">
                  {[...CARDS, ...(more ? MORE : [])].map((c) => (
                    <button key={c.kind} className="ox-card" onClick={() => open(c.kind)}>
                      <span className="ox-card-title">
                        {c.icon ? <c.icon size={18} /> : <KubernetesIcon kind={c.kind} size={18} />} {c.label.toUpperCase()}
                      </span>
                      <span className="ox-card-value">{compact(count.get(c.kind) ?? 0)}</span>
                    </button>
                  ))}
                </div>
                <button className="btn btn-sm ox-more" onClick={() => setMore((m) => !m)}>
                  {more ? <ChevronUp size={14} /> : <ChevronDown size={14} />} {more ? 'View fewer resources' : 'View more resources'}
                </button>
              </div>
            </section>

            <h2 className="ox-section-title">Your autoscaling operations at a glance</h2>
            <div className="ox-glance">
              <section className="ox-glance-card">
                <h3>
                  COST OF UNUSED RESOURCES <span className="ox-pill">LAST 30 DAYS</span>
                </h3>
                <div className="ox-glance-row">
                  <div>
                    <div className="ox-glance-label">IDLE COST</div>
                    <div className="ox-glance-big">—</div>
                  </div>
                  <div className="ox-glance-right">
                    <div className="ox-glance-label">TOTAL COST</div>
                    <div className="ox-glance-mid">—</div>
                  </div>
                </div>
                <p className="muted small">Costs need cloud billing data, which ObserveCraft doesn't collect yet.</p>
              </section>
              <section className="ox-glance-card">
                <h3>
                  WORKLOADS BY AUTOSCALING STATUS <span className="ox-pill">LAST 30 DAYS</span>
                </h3>
                <div className="ox-glance-row">
                  <div>
                    <div className="ox-glance-label">READY TO AUTOSCALE</div>
                    <div className="ox-glance-big">{workloads.length ? `${Math.round((ready / workloads.length) * 100)} %` : '—'}</div>
                  </div>
                  <div className="ox-glance-right">
                    <div className="ox-glance-label">TOTAL WORKLOADS</div>
                    <div className="ox-glance-mid">{compact(totalWorkloads)}</div>
                  </div>
                </div>
                <p className="muted small">Deployments that set CPU requests and have no Horizontal Pod Autoscaler yet.</p>
              </section>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
