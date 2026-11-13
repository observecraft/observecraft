// The service page's primary tags as span search terms (pure: shared by the client, the mock and tests).
import { escapeQuoted } from '../../lib/searchQuery';

/** Primary tags of the service page: Kubernetes cluster and namespace, as Datadog's tags name them. */
export interface PrimaryTags {
  cluster?: string | null;
  namespace?: string | null;
}

const set = (v: string | null | undefined): v is string => !!v && v !== '*';

/** `kube_cluster_name:"prod-eu-1" kube_namespace:"shop"`, the tags that are set. */
export function primaryTagQuery(t: PrimaryTags): string {
  return [set(t.cluster) && `kube_cluster_name:"${escapeQuoted(t.cluster)}"`, set(t.namespace) && `kube_namespace:"${escapeQuoted(t.namespace)}"`].filter(Boolean).join(' ');
}
