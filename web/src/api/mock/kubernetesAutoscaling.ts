// Kubernetes → Autoscaling in mock mode, built from the mock inventory: production runs the Cluster
// Agent's workload autoscaling (two DatadogPodAutoscalers, one on ObserveCraft's recommender, one left
// behind by a deleted workload), staging has an HPA and a cluster autoscaler.
import type {
  AutoscalingOverview,
  AutoscalingProfile,
  AutoscalingSettings,
  ClusterScaling,
  KubeAutoscaler,
  KubeResource,
  ProfileInput,
  Rollout,
  SavedProfile,
  StoredRecommendation,
  WorkloadProblem,
  WorkloadRecommendation,
} from '../kubernetes/client';
import { inventory } from './kubernetes';

const GIB = 1024 ** 3;
const MIB = 1024 ** 2;
const HOURS = 730;

let settings: AutoscalingSettings = { cpuHourPrice: 0.0316, memoryGibHourPrice: 0.0042 };
let saved: SavedProfile[] = [{ id: '3f2c1e8a-5b7d-4c9e-8a1f-2d6b0c4e7a91', name: 'batch-vertical', template: { applyPolicy: { mode: 'Apply', update: { strategy: 'Auto' } } } }];

const monthly = (cpu: number, mem: number) => (cpu * settings.cpuHourPrice + (mem / GIB) * settings.memoryGibHourPrice) * HOURS;
const tag = (r: KubeResource, key: string) => r.tags.find((t) => t.startsWith(`${key}:`))?.slice(key.length + 1);
const up = (v: number, step: number) => Math.ceil(v / step - 1e-6) * step;

const BALANCE_TEMPLATE = {
  applyPolicy: {
    mode: 'Apply',
    scaleUp: { strategy: 'Max', stabilizationWindowSeconds: 130, rules: [{ type: 'Percent', value: 50, periodSeconds: 120 }] },
    scaleDown: { strategy: 'Max', stabilizationWindowSeconds: 600, rules: [{ type: 'Percent', value: 20, periodSeconds: 1200 }] },
  },
  constraints: { minReplicas: 2 },
  objectives: [{ type: 'PodResource', podResource: { name: 'cpu', value: { type: 'Utilization', utilization: 70 } } }],
};

function autoscalers(rows: KubeResource[]): KubeAutoscaler[] {
  const base = { owner: 'Local', profile: null, targetKind: 'Deployment', podsPending: 0, podsFailing: 0, recommender: null, problems: [] as string[], currentReplicas: null, desiredReplicas: null };
  const pods = (cluster: string, name: string) => rows.filter((p) => p.kind === 'pods' && p.cluster === cluster && tag(p, 'kube_deployment') === name);
  const running = (cluster: string, name: string) => pods(cluster, name).filter((p) => p.status === 'Running').length;
  return [
    {
      ...base,
      uid: 'dpa-storefront-api',
      cluster: 'shop-prod-eu',
      namespace: 'checkout',
      name: 'storefront-api',
      kind: 'DatadogPodAutoscaler',
      createdAt: Date.now() - 9 * 86400e3,
      targetName: 'storefront-api',
      targetExists: true,
      horizontal: true,
      vertical: true,
      minReplicas: 2,
      maxReplicas: 8,
      currentReplicas: 3,
      desiredReplicas: 3,
      podsRunning: running('shop-prod-eu', 'storefront-api'),
      recommender: 'https://intake.observecraft.example.com/api/v2/autoscaling/recommend?api_key=<INGEST_API_KEY>',
    },
    {
      ...base,
      uid: 'dpa-api-gateway',
      cluster: 'shop-prod-eu',
      namespace: 'checkout',
      name: 'api-gateway',
      kind: 'DatadogPodAutoscaler',
      profile: 'datadog-optimize-balance',
      createdAt: Date.now() - 4 * 86400e3,
      targetName: 'api-gateway',
      targetExists: true,
      horizontal: true,
      vertical: false,
      minReplicas: 2,
      currentReplicas: 3,
      desiredReplicas: 3,
      podsRunning: running('shop-prod-eu', 'api-gateway'),
    },
    {
      ...base,
      uid: 'dpa-legacy-reports',
      cluster: 'shop-prod-eu',
      namespace: 'catalog',
      name: 'legacy-reports',
      kind: 'DatadogPodAutoscaler',
      createdAt: Date.now() - 40 * 86400e3,
      targetName: 'legacy-reports',
      targetExists: false,
      horizontal: false,
      vertical: true,
      podsRunning: 0,
      problems: ['Target Deployment legacy-reports not found'],
    },
    {
      ...base,
      uid: 'hpa-order-service',
      cluster: 'shop-staging',
      namespace: 'checkout',
      name: 'order-service',
      kind: 'HorizontalPodAutoscaler',
      owner: null,
      createdAt: Date.now() - 20 * 86400e3,
      targetName: 'order-service',
      targetExists: true,
      horizontal: true,
      vertical: false,
      minReplicas: 1,
      maxReplicas: 4,
      currentReplicas: 2,
      desiredReplicas: 2,
      podsRunning: running('shop-staging', 'order-service'),
    },
  ];
}

function recommendations(rows: KubeResource[], scalers: KubeAutoscaler[]): WorkloadRecommendation[] {
  return rows
    .filter((r) => r.kind === 'deployments')
    .map((w) => {
      const pods = rows.filter((p) => p.kind === 'pods' && p.cluster === w.cluster && tag(p, 'kube_deployment') === w.name && p.status === 'Running');
      const replicas = w.desired ?? pods.length;
      const template = pods[0]?.containers ?? [];
      // A day of usage: the current usage stands for the average, the peak is a third higher.
      const avgCpu = pods.reduce((s, p) => s + (p.cpu?.usage ?? 0), 0) / Math.max(1, pods.length);
      const avgMem = pods.reduce((s, p) => s + (p.memory?.usage ?? 0), 0) / Math.max(1, pods.length);
      let cpuIdle = 0;
      let memUnused = 0;
      let requestsCost = 0;
      let usageCost = 0;
      let savings = 0;
      const containers = template.map((c) => {
        const cpuReq = c.cpu.requests ?? null;
        const memReq = c.memory.requests ?? null;
        const hasUsage = pods.length > 0 && avgCpu > 0;
        const recCpu = hasUsage ? Math.max(0.01, up(avgCpu * 1.3 * 1.15, 0.001)) : null;
        const recMem = hasUsage ? Math.max(32 * MIB, up(avgMem * 1.25 * 1.15, MIB)) : null;
        cpuIdle += replicas * Math.max(0, (cpuReq ?? 0) - avgCpu);
        memUnused += replicas * Math.max(0, (memReq ?? 0) - avgMem);
        requestsCost += replicas * monthly(cpuReq ?? 0, memReq ?? 0);
        usageCost += replicas * monthly(avgCpu, avgMem);
        if (recCpu != null && recMem != null && cpuReq != null && memReq != null) savings += replicas * (monthly(cpuReq, memReq) - monthly(recCpu, recMem));
        return {
          name: c.name,
          cpuRequest: cpuReq,
          memoryRequest: memReq,
          cpuP95: hasUsage ? avgCpu * 1.3 : null,
          cpuAvg: hasUsage ? avgCpu : null,
          memoryMax: hasUsage ? avgMem * 1.25 : null,
          memoryAvg: hasUsage ? avgMem : null,
          recommendedCpu: recCpu,
          recommendedMemory: recMem,
        };
      });
      const scaler = scalers.find((a) => a.cluster === w.cluster && a.namespace === w.namespace && a.targetName === w.name);
      const notReady: string[] = [];
      if (!pods.length || avgCpu === 0) notReady.push('No usage metrics');
      if (template.some((c) => c.memory.limits == null) && w.name === 'workflow-engine') notReady.push('Less than 6 hours of usage');
      if (scaler) notReady.push(`Scaled by ${scaler.kind} ${scaler.name}`);
      return {
        uid: w.uid,
        cluster: w.cluster,
        namespace: w.namespace ?? '',
        kind: 'Deployment',
        name: w.name,
        service: tag(w, 'service') ?? null,
        replicas,
        containers,
        cpuIdle,
        memoryUnused: memUnused,
        requestsCost,
        usageCost,
        idleCost: monthly(cpuIdle, memUnused),
        projectedSavings: Math.max(0, savings),
        notReady,
        agentSetup: w.cluster === 'shop-prod-eu',
        autoscaler: scaler ? `${scaler.kind}/${scaler.name}` : null,
        profile: w.name === 'api-gateway' ? 'datadog-optimize-balance' : null,
      };
    });
}

function clusters(rows: KubeResource[], workloads: WorkloadRecommendation[]): ClusterScaling[] {
  return rows
    .filter((r) => r.kind === 'clusters')
    .map((c) => {
      const nodes = rows.filter((n) => n.kind === 'nodes' && n.cluster === c.name);
      const pods = rows.filter((p) => p.kind === 'pods' && p.cluster === c.name);
      const cpuCapacity = nodes.reduce((s, n) => s + (n.cpu?.capacity ?? 0), 0);
      const memoryCapacity = nodes.reduce((s, n) => s + (n.memory?.capacity ?? 0), 0);
      const cpuRequests = pods.reduce((s, p) => s + (p.cpu?.requests ?? 0), 0);
      const memoryRequests = pods.reduce((s, p) => s + (p.memory?.requests ?? 0), 0);
      const largest = nodes.reduce((m, n) => [Math.max(m[0], n.cpu?.capacity ?? 0), Math.max(m[1], n.memory?.capacity ?? 0)], [0, 0]);
      const idle = [Math.max(0, cpuCapacity - cpuRequests), Math.max(0, memoryCapacity - memoryRequests)];
      const own = workloads.filter((w) => w.cluster === c.name);
      return {
        cluster: c.name,
        autoscaling: c.name === 'shop-staging' ? 'cluster_autoscaler' : 'none',
        nodes: nodes.length,
        cpuCapacity,
        memoryCapacity,
        cpuRequests,
        memoryRequests,
        clusterIdleCost: monthly(idle[0], idle[1]),
        workloadIdleCost: own.reduce((s, w) => s + w.idleCost, 0),
        usageCost: own.reduce((s, w) => s + w.usageCost, 0),
        projectedWorkloadSavings: own.reduce((s, w) => s + w.projectedSavings, 0),
        projectedClusterSavings: monthly(Math.max(0, idle[0] - largest[0]), Math.max(0, idle[1] - largest[1])),
      };
    });
}

function profiles(): AutoscalingProfile[] {
  const builtin = ['datadog-optimize-balance', 'datadog-optimize-cost', 'datadog-optimize-performance'].map((name) => ({
    name,
    source: 'builtin' as const,
    id: null,
    clusters: ['shop-prod-eu'],
    workloads: name === 'datadog-optimize-balance' ? 1 : 0,
    template: BALANCE_TEMPLATE,
  }));
  const own = saved.map((p) => ({ name: p.name, source: 'saved' as const, id: p.id, clusters: [], workloads: 0, template: p.template }));
  return [...own, ...builtin].sort((a, b) => a.name.localeCompare(b.name));
}

export async function getAutoscaling(): Promise<AutoscalingOverview> {
  const rows = inventory();
  const scalers = autoscalers(rows);
  const workloads = recommendations(rows, scalers);
  const cs = clusters(rows, workloads);
  const answers: StoredRecommendation[] = [
    {
      cluster: 'shop-prod-eu',
      namespace: 'checkout',
      kind: 'Deployment',
      name: 'storefront-api',
      currentReplicas: 3,
      targetReplicas: 3,
      reason: 'cpu utilization 64% for a target of 70%: 3 → 3 replicas',
      error: null,
      updatedAt: Date.now() - 20_000,
    },
  ];
  return {
    settings,
    summary: {
      usageCost: cs.reduce((s, c) => s + c.usageCost, 0),
      workloadIdleCost: cs.reduce((s, c) => s + c.workloadIdleCost, 0),
      clusterIdleCost: cs.reduce((s, c) => s + c.clusterIdleCost, 0),
      workloadSavings: cs.reduce((s, c) => s + c.projectedWorkloadSavings, 0),
      clusterSavings: cs.reduce((s, c) => s + c.projectedClusterSavings, 0),
      workloads: workloads.length,
      unscaledWorkloads: workloads.filter((w) => !w.autoscaler).length,
      clusters: cs.length,
      unscaledClusters: cs.filter((c) => c.autoscaling === 'none').length,
    },
    clusters: cs,
    workloads,
    autoscalers: scalers,
    profiles: profiles(),
    recommendations: answers,
  };
}

export async function updateAutoscalingSettings(next: AutoscalingSettings): Promise<AutoscalingSettings> {
  settings = next;
  return next;
}

export async function createAutoscalingProfile(input: ProfileInput): Promise<SavedProfile> {
  if (saved.some((p) => p.name === input.name)) throw new Error('a profile with this name already exists');
  const p = { id: crypto.randomUUID(), name: input.name, template: input.template };
  saved = [...saved, p];
  return p;
}

export async function updateAutoscalingProfile(id: string, input: ProfileInput): Promise<SavedProfile> {
  const p = { id, name: input.name, template: input.template };
  saved = saved.map((x) => (x.id === id ? p : x));
  return p;
}

export async function deleteAutoscalingProfile(id: string): Promise<void> {
  saved = saved.filter((p) => p.id !== id);
}

// ---------------------------------------------------------------- remediation and rollouts

const FIXES: Record<string, string> = {
  crash_loop_back_off: 'The container exits right after starting. Read its last logs (kubectl logs --previous), then check the command, configuration and the dependencies it needs at startup.',
  oom_killed: 'The container used more memory than its limit. Raise the memory limit (and request) to the peak it needs, or find what grows its memory.',
  image_pull: "The node can't pull the image. Check the image name and tag, that it exists in the registry, and the pod's imagePullSecrets.",
};

/** The mock inventory's crash-looping pods, plus an out-of-memory kill and an image pull failure. */
export async function listWorkloadProblems(): Promise<WorkloadProblem[]> {
  const rows = inventory();
  const crashing = rows.filter((p) => p.kind === 'pods' && p.status === 'CrashLoopBackOff');
  const total = (cluster: string, name: string) => rows.filter((p) => p.kind === 'pods' && p.cluster === cluster && tag(p, 'kube_deployment') === name).length;
  const problem = (cluster: string, namespace: string, name: string, issue: WorkloadProblem['issue'], label: string, pods: string[], started: number): WorkloadProblem => ({
    cluster,
    namespace,
    kind: 'Deployment',
    name,
    uid: `${cluster}-dep-${name}`,
    service: name,
    team: namespace,
    issue,
    issueLabel: label,
    remediation: FIXES[issue],
    podsAffected: pods.length,
    podsTotal: Math.max(pods.length, total(cluster, name)),
    started,
    pods,
  });
  return [
    ...crashing.map((p) => problem(p.cluster, p.namespace ?? '', tag(p, 'kube_deployment') ?? p.name, 'crash_loop_back_off', 'CrashLoopBackOff', [p.name], p.createdAt ?? Date.now())),
    problem('shop-prod-eu', 'payments', 'workflow-engine', 'oom_killed', 'OOMKilled', ['workflow-engine-7d9f8c6b5-xkqzt0w'], Date.now() - 50 * 60e3),
    problem('shop-staging', 'checkout', 'notification-worker', 'image_pull', 'Image pull failure', ['notification-worker-7d9f8c6b5-xkqzt0w'], Date.now() - 12 * 60e3),
  ];
}

export async function listRollouts(): Promise<Rollout[]> {
  const now = Date.now();
  return inventory()
    .filter((r) => r.kind === 'deployments')
    .map((d, i) => {
      const desired = d.desired ?? 1;
      const rolling = d.cluster === 'shop-prod-eu' && d.name === 'storefront-api';
      const stalled = d.cluster === 'shop-staging' && d.name === 'notification-worker';
      const started = rolling ? now - 95_000 : stalled ? now - 14 * 60e3 : now - (i + 1) * 5 * 3600e3;
      const finished = rolling || stalled ? null : started + 80_000 + i * 7_000;
      return {
        uid: d.uid,
        cluster: d.cluster,
        namespace: d.namespace ?? '',
        name: d.name,
        revision: 4 + (i % 5),
        status: (rolling ? 'in_progress' : stalled ? 'stalled' : 'complete') as Rollout['status'],
        desired,
        updated: rolling ? Math.max(1, desired - 1) : stalled ? 0 : desired,
        ready: d.available ?? desired,
        available: d.available ?? desired,
        image: d.images[0] ?? null,
        started,
        finished,
        durationSeconds: ((finished ?? now) - started) / 1000,
        message: stalled ? `ReplicaSet "${d.name}-5c6d7e8f9" has timed out progressing.` : null,
      };
    })
    .sort((a, b) => b.started - a.started);
}
