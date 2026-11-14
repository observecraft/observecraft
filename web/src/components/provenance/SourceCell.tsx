// Where a resource comes from (its provenance: Terraform tags, Argo CD, OCI labels, git metadata), as a table cell,
// and the "No source" filter the resource tables share.
import { Box, FileCode2, GitBranch, Layers, Tag } from 'lucide-react';
import type { Provenance, ProvenanceVia } from '../../api/tagging/types';
import './provenance.css';

export const VIA_LABELS: Record<ProvenanceVia, string> = {
  tag: 'Tags',
  state: 'Terraform state',
  argocd: 'Argo CD',
  oci: 'OCI labels',
  git_metadata: 'Git metadata',
};

const KIND_LABELS: Record<Provenance['kind'], string> = {
  terraform: 'Terraform',
  helm: 'Helm',
  kustomize: 'Kustomize',
  manifest: 'Manifests',
  dockerfile: 'Dockerfile',
  code: 'Code',
};

function ViaIcon({ via }: { via: ProvenanceVia }) {
  const Icon = via === 'argocd' ? Layers : via === 'oci' ? Box : via === 'git_metadata' ? GitBranch : via === 'state' ? FileCode2 : Tag;
  return <Icon size={13} aria-hidden />;
}

/** One line: the repository (or the Argo CD Application), the path, and how it was found. */
export function SourceCell({ provenance: p }: { provenance?: Provenance | null }) {
  if (!p) return <span className="prov-none">No source</span>;
  const where = p.repo ? p.repo.split('/').slice(-2).join('/') : p.application ? `app ${p.application}` : '—';
  const title = [
    `${KIND_LABELS[p.kind]} · found through ${VIA_LABELS[p.via]}${p.confidence === 'inferred' ? ' (inferred)' : ''}`,
    p.repo && `Repository: ${p.repo}`,
    p.path && `Path: ${p.path}`,
    p.ref && `Ref: ${p.ref}`,
    p.address && `Address: ${p.address}`,
    p.application && `Argo CD Application: ${p.application}`,
    !p.repo && p.application && 'The Application object isn’t collected, so its repository is unknown.',
  ]
    .filter(Boolean)
    .join('\n');
  return (
    <span className="prov-cell" title={title}>
      <span className={`prov-via ${p.via}`}>
        <ViaIcon via={p.via} />
      </span>
      <span className="prov-where mono">
        {where}
        {p.path && <span className="prov-path">/{p.path.replace(/^\/+/, '')}</span>}
      </span>
      {p.confidence === 'inferred' && <span className="prov-inferred">inferred</span>}
    </span>
  );
}

/** The "No source" switch of a resource table. */
export function NoSourceToggle({ checked, onChange, count }: { checked: boolean; onChange: (v: boolean) => void; count?: number }) {
  return (
    <label className="prov-toggle">
      <button type="button" role="switch" aria-checked={checked} className={`switch${checked ? ' on' : ''}`} onClick={() => onChange(!checked)} />
      No source{count != null && <span className="muted"> ({count.toLocaleString()})</span>}
    </label>
  );
}
