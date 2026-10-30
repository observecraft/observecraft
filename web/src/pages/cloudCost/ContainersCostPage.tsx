import { useMemo } from 'react';
import { Link } from 'react-router-dom';
import { CircleDollarSign, ExternalLink } from 'lucide-react';
import { Loading } from '../../components/ui/AsyncState';
import { DAY, today, useCostTags } from './costModel';
import { CostBoardPage } from './CostBoardPage';

/**
 * Summarize → Containers: the costs tagged with a Kubernetes cluster (`kube_cluster_name`), by cluster,
 * namespace and workload; until there are any, how to get them.
 */
export function ContainersCostPage() {
  const now = today();
  const range = useMemo(() => ({ from: now - 30 * DAY, to: now + DAY - 1 }), [now]);
  const tags = useCostTags(range, 'kube_cluster_name:*');
  if (tags.isPending) return <Loading label="Loading container costs…" />;
  if (tags.data?.some((t) => t.key === 'kube_cluster_name')) return <CostBoardPage tab="containers" />;
  return (
    <div className="ccm-page">
      <div className="ccm-enable">
        <CircleDollarSign size={56} strokeWidth={1.4} />
        <h2>No container costs yet</h2>
        <p>
          Costs tagged with their Kubernetes cluster (<code>kube_cluster_name</code>, and <code>kube_namespace</code> or <code>kube_deployment</code> for workloads) show here by cluster, namespace and
          workload. Tag the billed resources with their cluster in your cloud bills, or upload custom costs files with these tags.
        </p>
        <Link className="btn btn-primary" to="/kubernetes">
          Kubernetes monitoring <ExternalLink size={14} />
        </Link>
      </div>
    </div>
  );
}
