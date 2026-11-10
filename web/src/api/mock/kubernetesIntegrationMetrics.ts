// Mock metrics of the Kubernetes integrations the mock shows installed (ingress-nginx, CoreDNS, KEDA, Karpenter,
// Temporal, vLLM, DCGM, the control plane, Argo Workflows and Rollouts, Istio, Envoy): their tags' values and the values
// that read as theirs (object counts, 0/1 states, versions), on the demo's clusters. Read by coreMetrics.ts.

type Tags = Record<string, string>;
type Rand = () => number;
type Override = { base: (e: Tags, r: Rand) => number; noise: number; integer?: boolean; max?: number };

/** A value between `lo` and `hi` per instance, varying by `noise`. */
const between = (lo: number, hi: number, noise = 0.15, integer = false): Override => ({ base: (_e, r) => lo + r() * (hi - lo), noise, ...(integer ? { integer } : {}) });
/** Always `v` (versions, info gauges, configured limits). */
const fixed = (v: number): Override => ({ base: () => v, noise: 0, integer: Number.isInteger(v) });
/** Usually 0, sometimes up to `hi` (errors, retries, restarts). */
const rare = (hi: number, share = 0.25): Override => ({ base: (_e, r) => (r() < share ? 1 + r() * (hi - 1) : 0), noise: 0.6, integer: true });

export const INFRA_VALUES: Record<string, Record<string, string[]>> = {
  'nginx-ingress-controller': {
    state: ['active', 'reading', 'writing', 'waiting'],
    controller_pod: ['ingress-nginx-controller-7c9d8b6f5-4qmzx', 'ingress-nginx-controller-7c9d8b6f5-8hjkt', 'ingress-nginx-controller-7c9d8b6f5-r2vnc', 'ingress-nginx-controller-5b8f7d9c4-x7lpw'],
  },
};

export const INFRA_OVERRIDES: Record<string, Override> = {
  'nginx_ingress.nginx.connections.current': between(40, 160, 0.2, true),
  'nginx_ingress.nginx.process.count': fixed(5),
  'nginx_ingress.nginx.bytes.read': between(4e6, 40e6, 0.3),
  'nginx_ingress.nginx.bytes.write': between(30e6, 300e6, 0.3),
  'nginx_ingress.nginx.mem.resident': between(90e6, 240e6, 0.05),
  'nginx_ingress.nginx.mem.virtual': between(420e6, 680e6, 0.02),
  'nginx_ingress.nginx.cpu.time': between(0.05, 0.4, 0.3),
  'nginx_ingress.controller.cpu.time': between(0.01, 0.08, 0.3),
  'nginx_ingress.controller.mem.resident': between(60e6, 140e6, 0.05),
  'nginx_ingress.controller.mem.virtual': between(760e6, 1.3e9, 0.02),
  'nginx_ingress.controller.reload.success': rare(2, 0.1),
};
