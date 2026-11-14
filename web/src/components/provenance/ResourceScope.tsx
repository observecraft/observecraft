// A resource's account or cluster as a table cell: the provider's (or Kubernetes') logo and the account's display name,
// one line with an ellipsis, the raw ids in the tooltip.
import { useMemo } from 'react';
import type { TaggedResource } from '../../api/tagging/types';
import { BrandLogo } from '../brand/BrandLogo';
import { KubernetesLogo } from '../brand/KubernetesIcon';
import { useCloudSummary } from '../../pages/security/cloud/queries';
import { resourceScope } from './scope';
import './provenance.css';

/** `provider:account` → the account's display name, from Cloud Security's list of accounts. */
export function useAccountNames(): Map<string, string> {
  const q = useCloudSummary();
  return useMemo(() => new Map((q.data?.coverage.accounts ?? []).filter((a) => a.name).map((a) => [`${a.provider}:${a.id}`, a.name])), [q.data]);
}

export function ResourceScope({ resource, names }: { resource: Pick<TaggedResource, 'target' | 'provider' | 'account' | 'region' | 'cluster' | 'namespace'>; names: Map<string, string> }) {
  const s = resourceScope(resource, names);
  if (!s.logo) return <span className="muted">{s.text}</span>;
  return (
    <span className="prov-scope" title={s.title}>
      {s.logo === 'kubernetes' ? <KubernetesLogo size={14} /> : <BrandLogo name={s.logo} size={14} maxRatio={1.7} />}
      <span className="prov-ellipsis">{s.text}</span>
    </span>
  );
}
