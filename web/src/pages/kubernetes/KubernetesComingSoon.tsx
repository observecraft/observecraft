import { OrchestrationHeader } from './OrchestrationHeader';
import '../infrastructure/infrastructure.css';
import './kubernetes.css';

/** Kubernetes tabs that need data ObserveCraft doesn't collect yet (costs, autoscaler and rollout state). */
export function KubernetesComingSoon({ tab }: { tab: string }) {
  return (
    <div className="hosts-page ox-page">
      <OrchestrationHeader product="kubernetes" active={tab} />
      <div className="placeholder-page">{tab} hasn't been built yet.</div>
    </div>
  );
}
