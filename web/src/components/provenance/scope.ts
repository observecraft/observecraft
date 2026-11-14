// Where a resource lives, as one line: its cloud account's display name (the Azure subscription's or Google Cloud
// project's name, else the id) or its Kubernetes cluster and namespace, with the raw ids in the title. Pure:
// scripts/tagging-policies.test.mjs.
import type { TaggedResource } from '../../api/tagging/types';

export interface ScopeView {
  /** BrandLogo key (aws, azure, googlecloud) or kubernetes. */
  logo: 'aws' | 'azure' | 'googlecloud' | 'kubernetes' | null;
  text: string;
  title: string;
}

const PROVIDERS: Record<string, { logo: ScopeView['logo']; label: string; unit: string }> = {
  aws: { logo: 'aws', label: 'AWS', unit: 'account' },
  azure: { logo: 'azure', label: 'Azure', unit: 'subscription' },
  gcp: { logo: 'googlecloud', label: 'Google Cloud', unit: 'project' },
};

/** `names` maps `provider:account` to the account's display name. */
export function resourceScope(r: Pick<TaggedResource, 'target' | 'provider' | 'account' | 'region' | 'cluster' | 'namespace'>, names: Map<string, string>): ScopeView {
  if (r.provider && r.account) {
    const p = PROVIDERS[r.provider];
    const name = names.get(`${r.provider}:${r.account}`);
    const title = [`${p?.label ?? r.provider} ${p?.unit ?? 'account'} ${r.account}`, r.region].filter(Boolean).join(' · ');
    return { logo: p?.logo ?? null, text: name || r.account, title };
  }
  if (r.cluster) {
    const text = [r.cluster, r.namespace].filter(Boolean).join(' · ');
    return { logo: 'kubernetes', text, title: `Kubernetes cluster ${r.cluster}${r.namespace ? ` · namespace ${r.namespace}` : ''}` };
  }
  return { logo: null, text: '—', title: '' };
}
