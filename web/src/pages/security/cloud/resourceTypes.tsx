// The icons of Cloud Security's resource types ([`resourceKind`] names them).
import { Box, FolderGit2, Server } from 'lucide-react';
import { INTEGRATION_LOGOS } from '../../../assets/integrationLogos.generated';
import { BrandLogo, hasBrandLogo } from '../../../components/brand/BrandLogo';
import { KubernetesIcon } from '../../../components/brand/KubernetesIcon';
import { ecosystemLogo, osLogo, resourceKind, type Kind } from './resourceKinds';

export { resourceKind };

function KindIcon({ kind, size }: { kind: Kind; size: number }) {
  // Kubernetes kinds: the Kubernetes project's icon of the kind (Pod, DaemonSet), else its logo.
  if (kind.kube) return <KubernetesIcon kind={kind.kube} size={size} title={kind.name} />;
  const key = kind.logo;
  if (key && hasBrandLogo(key)) return <BrandLogo name={key} size={size} title={kind.name} maxRatio={1.7} />;
  if (key && key in INTEGRATION_LOGOS) return <img className="logo-file" src={`/images/integrations/logos/${INTEGRATION_LOGOS[key]}`} alt="" loading="lazy" width={size} height={size} />;
  if (kind.fallback === 'aws' || key === 'aws') return <BrandLogo name="aws" size={size} title={kind.name} maxRatio={1.7} />;
  if (kind.fallback === 'azure') return <BrandLogo name="azure" size={size} title={kind.name} />;
  if (kind.fallback === 'gcp') return <BrandLogo name="googlecloud" size={size} title={kind.name} />;
  if (kind.fallback === 'repository') return <FolderGit2 size={size} />;
  if (kind.fallback === 'host') return <Server size={size} />;
  return <Box size={size} />;
}

/** The icon of a resource type alone (facet values). */
export function ResourceTypeIcon({ type, resource, size = 14 }: { type: string; resource?: string; size?: number }) {
  return (
    <span className="csm-rtype-icon" style={{ minWidth: size, height: size }}>
      <KindIcon kind={resourceKind(type, resource)} size={size} />
    </span>
  );
}

/** A resource type with its icon: EC2 Security Group, IAM User, DaemonSet. */
export function ResourceType({ type, resource, size = 16 }: { type: string; resource?: string; size?: number }) {
  const kind = resourceKind(type, resource);
  return (
    <span className="csm-rtype" title={type}>
      <span className="csm-rtype-icon" style={{ minWidth: size, height: size }}>
        <KindIcon kind={kind} size={size} />
      </span>
      {kind.name}
    </span>
  );
}

/** A host's or image's distribution logo (Debian, Ubuntu, Alpine, Red Hat…), nothing when unknown. */
export function OsIcon({ os, size = 14 }: { os: string; size?: number }) {
  const key = osLogo(os);
  return key && hasBrandLogo(key) ? <BrandLogo name={key} size={size} title={os} /> : null;
}

/** A package's ecosystem logo (npm, Maven, PyPI…, or the distribution of an OS package). */
export function EcosystemIcon({ ecosystem, size = 14 }: { ecosystem: string; size?: number }) {
  const key = ecosystemLogo(ecosystem);
  return key && hasBrandLogo(key) ? <BrandLogo name={key} size={size} title={ecosystem} /> : null;
}
