// Integrations → Kubernetes: the clusters sending data and where it comes from, Helm values for the
// official Datadog chart or the observecraft-agent chart, and the Kubernetes Agent checks.
import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { Check, ChevronRight, Info } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { listKubeClusters, type KubeClusterSummary, type KubeObjectSource } from '../../../api/kubernetes/client';
import { coreCatalog } from '../../../api/integrations/catalog';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { formatBytes, timeAgo } from '../../../lib/format';
import { IntegrationLogo } from '../IntegrationLogo';
import { datadogValues, helmCommands, otelValues, type KubeChart, type KubeInstallOptions } from './kubernetesSetup';
import '../../settings/settings.css';
import '../integrations.css';
import '../aws/aws.css';

const DESCRIPTION =
  'Kubernetes runs containerized workloads across a cluster of nodes. Install the Datadog Agent with its Helm chart (or the OpenTelemetry Collector) to collect metrics, logs, traces, live containers and every Kubernetes object, and to autoscale workloads with the Cluster Agent.';

const SOURCES: Record<KubeObjectSource, string> = { datadog_cluster_agent: 'Datadog Cluster Agent', opentelemetry: 'OpenTelemetry Collector' };

const TABS = [
  { id: 'clusters', label: 'Clusters' },
  { id: 'install', label: 'Install' },
  { id: 'checks', label: 'Kubernetes Checks' },
] as const;
type Tab = (typeof TABS)[number]['id'];

export function KubernetesIntegrationPage() {
  const [params, setParams] = useSearchParams();
  const tab: Tab = TABS.find((t) => t.id === params.get('tab'))?.id ?? 'clusters';
  const clusters = useQuery({ queryKey: ['kubernetes', 'clusters'], queryFn: listKubeClusters, refetchInterval: 60_000 });
  const installed = (clusters.data?.length ?? 0) > 0;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>Kubernetes</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo="kubernetes" name="Kubernetes" size={72} />
        <div>
          <h1>Kubernetes</h1>
          <p>{DESCRIPTION}</p>
        </div>
        {installed && (
          <span className="btn aws-installed">
            <Check size={15} /> Installed
          </span>
        )}
      </header>
      <nav className="im-tabs aws-tabs">
        {TABS.map((t) => (
          <button key={t.id} className={tab === t.id ? 'active' : ''} onClick={() => setParams(t.id === 'clusters' ? {} : { tab: t.id })}>
            {t.label}
          </button>
        ))}
      </nav>
      {tab === 'install' ? (
        <Install />
      ) : tab === 'checks' ? (
        <Checks />
      ) : clusters.isPending ? (
        <Loading label="Loading clusters…" />
      ) : clusters.isError ? (
        <ErrorState error={clusters.error} onRetry={() => clusters.refetch()} />
      ) : clusters.data.length === 0 ? (
        <Empty>
          No cluster is sending data yet.{' '}
          <button className="link-btn" onClick={() => setParams({ tab: 'install' })}>
            Install the Agent
          </button>{' '}
          on a cluster; it appears here within a minute.
        </Empty>
      ) : (
        <Clusters clusters={clusters.data} />
      )}
    </div>
  );
}

function Clusters({ clusters }: { clusters: KubeClusterSummary[] }) {
  return (
    <table className="settings-table">
      <thead>
        <tr>
          <th>Cluster</th>
          <th>Status</th>
          <th>Nodes</th>
          <th>Pods</th>
          <th>Version</th>
          <th>CPU capacity</th>
          <th>Memory capacity</th>
          <th>Objects from</th>
          <th>Workload autoscaling</th>
          <th>Last seen</th>
        </tr>
      </thead>
      <tbody>
        {clusters.map((c) => (
          <tr key={c.name}>
            <td>
              <Link to={`/orchestration/explorer?resource=clusters&query=${encodeURIComponent(`kube_cluster_name:${c.name}`)}`}>{c.name}</Link>
            </td>
            <td>{c.status}</td>
            <td>{c.nodes}</td>
            <td>{c.pods}</td>
            <td>{c.version ?? '—'}</td>
            <td>{c.cpuCapacity != null ? `${c.cpuCapacity.toFixed(1)} cores` : '—'}</td>
            <td>{c.memoryCapacity != null ? formatBytes(c.memoryCapacity) : '—'}</td>
            <td>{c.sources.length ? c.sources.map((s) => SOURCES[s]).join(', ') : '—'}</td>
            <td>{c.autoscaling ? <Link to="/kubernetes/autoscaling">Installed</Link> : <span className="muted">Not installed</span>}</td>
            <td>{c.lastSeen ? timeAgo(c.lastSeen) : '—'}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

const defaultIntake = () => `${window.location.protocol}//${window.location.hostname}:4318`;

function Install() {
  const [chart, setChart] = useState<KubeChart>('datadog');
  const [opts, setOpts] = useState<KubeInstallOptions>({ clusterName: 'my-cluster', intakeUrl: defaultIntake(), logs: true, apm: true, autoscaling: true });
  const set = <K extends keyof KubeInstallOptions>(k: K, v: KubeInstallOptions[K]) => setOpts((o) => ({ ...o, [k]: v }));
  const values = useMemo(() => (chart === 'datadog' ? datadogValues(opts) : otelValues(opts)), [chart, opts]);
  return (
    <section className="aws-form">
      <p>
        Use Datadog’s official Helm chart, unchanged: the Agent on every node and the Cluster Agent, sending to this install. For clusters without the Datadog Agent, the observecraft-agent chart runs
        the OpenTelemetry Collector instead. Create the key secret with an ingest key from <Link to="/organization-settings/api-keys">API Keys</Link>.
      </p>
      <div className="aws-inline">
        <label className="aws-toggle">
          <input type="radio" checked={chart === 'datadog'} onChange={() => setChart('datadog')} />
          <span>Datadog Helm chart</span>
        </label>
        <label className="aws-toggle">
          <input type="radio" checked={chart === 'opentelemetry'} onChange={() => setChart('opentelemetry')} />
          <span>OpenTelemetry (observecraft-agent chart)</span>
        </label>
      </div>
      <label className="rt-field">
        <span>Cluster name</span>
        <input value={opts.clusterName} onChange={(e) => set('clusterName', e.target.value)} />
      </label>
      <label className="rt-field">
        <span>Intake URL, as the cluster reaches it</span>
        <input value={opts.intakeUrl} onChange={(e) => set('intakeUrl', e.target.value)} />
      </label>
      <div className="aws-inline">
        <label className="aws-toggle">
          <input type="checkbox" checked={opts.logs} onChange={(e) => set('logs', e.target.checked)} />
          <span>Container logs</span>
        </label>
        <label className="aws-toggle">
          <input type="checkbox" checked={opts.apm} onChange={(e) => set('apm', e.target.checked)} />
          <span>APM traces</span>
        </label>
        {chart === 'datadog' && (
          <label className="aws-toggle">
            <input type="checkbox" checked={opts.autoscaling} onChange={(e) => set('autoscaling', e.target.checked)} />
            <span>Workload autoscaling (Cluster Agent)</span>
          </label>
        )}
      </div>
      {chart === 'datadog' && opts.autoscaling && (
        <div className="aws-note">
          <Info size={16} />
          <span>
            Workload autoscaling needs Remote Configuration enabled in the Cluster Agent. The site is <code>observecraft.invalid</code>, so it never reaches Datadog. Horizontal scaling then works with
            the Cluster Agent’s local recommendations or ObserveCraft’s recommender (Autoscaling → Setup). Vertical recommendations are applied from the manifests ObserveCraft generates.
          </span>
        </div>
      )}
      <CodeBlock code={values} filename="values.yaml" />
      <CodeBlock code={helmCommands(chart).join('\n')} filename="install.sh" />
    </section>
  );
}

/** Agent checks of the cluster itself and of the tools commonly run in it (integrations-core). */
const KUBE_CHECK = /kube|k8s|eks|openshift|etcd|coredns|karpenter|cilium|istio|calico|argo|flux|cert_manager|external_dns|nginx_ingress/i;

function Checks() {
  const checks = useMemo(() => coreCatalog.integrations.filter((i) => i.configurable && KUBE_CHECK.test(i.check)), []);
  return (
    <section>
      <p className="muted">These checks run in the Datadog Agent; enable them from their tiles in the catalog. Their metrics feed the Kubernetes pages and dashboards.</p>
      <table className="settings-table">
        <thead>
          <tr>
            <th>Check</th>
            <th>Collects</th>
          </tr>
        </thead>
        <tbody>
          {checks.map((c) => (
            <tr key={c.id}>
              <td>
                <Link to={`/integrations?integration=${encodeURIComponent(c.id)}`}>{c.name}</Link>
              </td>
              <td className="muted">{c.description}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
