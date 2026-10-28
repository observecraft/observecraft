// Labels, colors and links of change tracking (Infrastructure → Changes).
import type { Change, ChangeType } from '../../api/changes/client';

export const CHANGE_TYPES: ChangeType[] = ['deployment', 'rollout', 'scale', 'config', 'feature_flag', 'crash'];

export const CHANGE_TYPE_LABELS: Record<ChangeType, string> = {
  deployment: 'Deployment',
  rollout: 'Kubernetes Rollout',
  scale: 'Scaling',
  config: 'Configuration',
  feature_flag: 'Feature Flag',
  crash: 'Pod Crash',
};

/** Chart color token of each change type (histogram bars and list markers). */
export const CHANGE_TYPE_TOKENS: Record<ChangeType, string> = {
  deployment: '--chart-1',
  rollout: '--chart-3',
  scale: '--chart-2',
  config: '--chart-4',
  feature_flag: '--chart-7',
  crash: '--chart-5',
};

const SOURCE_LABELS: Record<string, string> = { kubernetes: 'Kubernetes', apm: 'APM', ci: 'CI', events: 'Events', audit: 'Audit Trail' };

export function sourceLabel(source: string): string {
  return SOURCE_LABELS[source] ?? source;
}

const FACET_LABELS: Record<string, string> = {
  change_type: 'Change Type',
  source: 'Source',
  resource_type: 'Resource Type',
  service: 'Service',
  env: 'Environment',
  cluster: 'Cluster',
  namespace: 'Namespace',
  author: 'Author',
};

export function changeFacetLabel(key: string): string {
  return FACET_LABELS[key] ?? key;
}

/** The page of the changed object, when it has one. */
export function changeLink(c: Pick<Change, 'kubeKind' | 'kubeUid' | 'resourceType' | 'resource' | 'source'> & { details?: Record<string, string> }): string | null {
  if (c.kubeKind && c.kubeUid) return `/orchestration/explorer?${new URLSearchParams({ resource: c.kubeKind, panel: c.kubeUid })}`;
  if (c.resourceType === 'service') return `/apm/services/${encodeURIComponent(c.resource)}`;
  const monitor = c.resourceType === 'monitor' ? c.details?.target?.match(/^monitor:(\w+)$/) : null;
  if (monitor) return `/monitors/${monitor[1]}`;
  if (c.resourceType === 'host') return `/infrastructure?${new URLSearchParams({ host: c.resource })}`;
  return null;
}
