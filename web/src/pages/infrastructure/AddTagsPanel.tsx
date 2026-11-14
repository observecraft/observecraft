// Tag Coverage → "Add tags with Assist" (ADR-0008 decision 5): opens a merge request on the repository the person picks
// that adds the tags naming where a resource is defined: the AWS provider's default_tags (or a merged locals map),
// OCI labels in the image's Dockerfile, DD_GIT_* for services. Reviewed as a diff like a finding's fix.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Sparkles } from 'lucide-react';
import { addTags, getFix, remediationOf } from '../../api/remediation/client';
import type { TaggedResource } from '../../api/tagging/types';
import { FixPanel } from '../../components/security/RemediationCard';
import '../../components/security/remediation.css';

const busy = (s?: string | null) => s === 'queued' || s === 'running';

/** What the change adds, per target. */
export function addTagsHint(r: Pick<TaggedResource, 'target' | 'provider'>): string {
  if (r.target === 'image') return 'Adds the OCI source and revision labels to the image’s Dockerfile under the path.';
  if (r.target === 'service') return 'Assist adds DD_GIT_REPOSITORY_URL and DD_GIT_COMMIT_SHA to the service’s deployment under the path.';
  if (r.provider === 'azure' || r.provider === 'gcp') return 'Assist adds a default_tags local to the root module under the path and merges it into its resources’ tags.';
  return 'Adds managed-by, tf_repo and tf_path to the AWS provider’s default_tags in the root module under the path.';
}

export function AddTagsPanel({ resource }: { resource: TaggedResource }) {
  const qc = useQueryClient();
  const key = ['remediation', 'resource', resource.id];
  const latest = useQuery({ queryKey: key, queryFn: ({ signal }) => remediationOf({ kind: 'resource', id: resource.id }, signal) });
  const [repository, setRepository] = useState(resource.provenance?.repo ?? '');
  const [path, setPath] = useState(resource.provenance?.path ?? '');
  const start = useMutation({ mutationFn: () => addTags({ resourceId: resource.id, repository, path }), onSuccess: () => qc.invalidateQueries({ queryKey: key }) });
  const fixId = start.data?.id ?? latest.data?.fix?.id;
  const fix = useQuery({
    queryKey: ['remediation', 'fix', fixId],
    queryFn: ({ signal }) => getFix(fixId!, signal),
    enabled: !!fixId,
    refetchInterval: (q) => (busy(q.state.data?.status) ? 2000 : false),
  });
  if (resource.target === 'kubernetes')
    return (
      <p className="muted rem-fine">
        Argo CD writes its tracking annotation itself: set <code>application.resourceTrackingMethod: annotation</code> in argocd-cm and collect the Applications with the Cluster Agent (Policies has
        the snippet).
      </p>
    );
  return (
    <section className="rem-card tg-addtags" aria-label="Add tags with Assist">
      <p className="muted">{addTagsHint(resource)} You review the diff before the merge request is opened.</p>
      <div className="tg-addtags-form">
        <label>
          <span>Repository</span>
          <input className="settings-input" value={repository} placeholder="github.com/example-org/infra-terraform" onChange={(e) => setRepository(e.target.value)} />
        </label>
        <label>
          <span>Path</span>
          <input className="settings-input" value={path} placeholder={resource.target === 'image' ? '(repository root)' : 'envs/prod/storage'} onChange={(e) => setPath(e.target.value)} />
        </label>
        <button className="btn btn-primary btn-sm" disabled={!repository.trim() || start.isPending || busy(fix.data?.status)} onClick={() => start.mutate()}>
          <Sparkles size={13} /> {start.isPending ? 'Preparing…' : fixId ? 'Propose again' : 'Propose the change'}
        </button>
      </div>
      {start.error && <p className="rem-warn">{(start.error as Error).message}</p>}
      {fix.data && <FixPanel fix={fix.data} onChange={() => fix.refetch()} />}
    </section>
  );
}
