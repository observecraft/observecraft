import type { CatalogType } from '../../api/catalog/client';
import type { Cloud } from '../../api/infrastructure/types';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { TypeIcon } from './CatalogPanel';
import { CLOUD_LABEL, CloudIcon } from './hostIcons';

const CLOUDS = new Set(Object.keys(CLOUD_LABEL));

/** The platform mark of a resource type: Kubernetes for its objects, AWS for ECS, else the type icon (or the host's cloud). */
export function ResourceLogo({ type, cloud, size = 18 }: { type: CatalogType; cloud?: string | null; size?: number }) {
  if (type.startsWith('kube_')) return <BrandLogo name="kubernetes" size={size} title="Kubernetes" />;
  if (type.startsWith('ecs_')) return <BrandLogo name="aws" size={size} title="AWS" />;
  if (type === 'host' && cloud && cloud !== 'none' && CLOUDS.has(cloud)) return <CloudIcon cloud={cloud as Cloud} size={size} />;
  return <TypeIcon type={type} size={size} />;
}
