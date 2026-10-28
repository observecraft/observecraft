import { Fragment, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { Box, Boxes, Container, ExternalLink, FolderTree, Layers, Network, Server, Ship, X } from 'lucide-react';
import type { CatalogCategory, CatalogResource, CatalogType } from '../../api/catalog/client';
import type { Cloud } from '../../api/infrastructure/types';
import { formatLongDate, timeAgo } from '../../lib/format';
import { StatusPill } from '../kubernetes/columns';
import { CATEGORY_LABELS, catalogLink, TYPE_LABELS } from './catalogModel';
import { CLOUD_LABEL } from './hostIcons';

// The resource side panel of the catalog, also opened from Policies and Cloudcraft.

export const ATTRIBUTE_LABELS: Record<string, string> = {
  os: 'OS',
  distro: 'Distribution',
  collector: 'Collector',
  cpu_pct: 'CPU %',
  mem_pct: 'Memory %',
  apps: 'Integrations',
  node: 'Node',
  ready: 'Ready',
  owner: 'Owner',
  images: 'Images',
  version: 'Version',
  instance_type: 'Instance type',
  availability_zone: 'Availability zone',
  arch: 'Architecture',
  ip: 'IP',
  service_type: 'Service type',
  task_definition: 'Task definition',
  launch_type: 'Launch type',
  nodes: 'Nodes',
  pods: 'Pods',
  restarts: 'Restarts',
};

export function TypeIcon({ type, size = 15 }: { type: CatalogType; size?: number }) {
  const Icon =
    type === 'host' || type === 'kube_node'
      ? Server
      : type === 'kube_cluster' || type === 'ecs_cluster'
        ? Ship
        : type === 'kube_namespace'
          ? FolderTree
          : type === 'kube_pod' || type === 'ecs_task'
            ? Box
            : type === 'container'
              ? Container
              : type === 'kube_service' || type === 'kube_ingress' || type === 'ecs_service'
                ? Network
                : type === 'kube_persistent_volume' || type === 'kube_persistent_volume_claim'
                  ? Layers
                  : Boxes;
  return <Icon size={size} className="rc-type-icon" aria-hidden />;
}

/** A facet value as people read it (type and category labels). */
export function valueLabel(key: string, value: string): string {
  if (key === 'resource_type') return TYPE_LABELS[value as CatalogType] ?? value;
  if (key === 'resource_category') return CATEGORY_LABELS[value as CatalogCategory] ?? value;
  if (key === 'cloud') return CLOUD_LABEL[value as Cloud] ?? value;
  return value;
}

/** A catalog resource's details: overview, attributes and tags; `onFilter` narrows the page by a value. */
export function CatalogPanel({ resource: r, onClose, onFilter }: { resource: CatalogResource; onClose: () => void; onFilter: (k: string, v: string) => void }) {
  const navigate = useNavigate();
  const link = catalogLink(r);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !(e.target as HTMLElement).closest('input, textarea') && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const field = (label: string, key: string, value: string | null | undefined) => (
    <>
      <dt>{label}</dt>
      <dd>{value ? <button onClick={() => onFilter(key, value)}>{valueLabel(key, value)}</button> : '—'}</dd>
    </>
  );
  const attributes = Object.entries(r.attributes);
  return (
    <aside className="rc-panel" role="dialog" aria-label={`${TYPE_LABELS[r.resourceType]} ${r.name}`}>
      <header>
        <TypeIcon type={r.resourceType} size={18} />
        <span>
          {CATEGORY_LABELS[r.resourceCategory]} · {TYPE_LABELS[r.resourceType]}
        </span>
        <button className="rc-panel-close" onClick={onClose} title="Close">
          <X size={16} />
        </button>
      </header>
      <h2>{r.name}</h2>
      <div className="rc-panel-actions">
        {r.status && r.health && <StatusPill status={r.status} health={r.health} />}
        {link && (
          <button className="btn" onClick={() => navigate(link)}>
            <ExternalLink size={14} /> {r.resourceType === 'host' ? 'Open host' : r.kubeKind?.startsWith('ecs_') ? 'Open in ECS Explorer' : 'Open in Kubernetes Explorer'}
          </button>
        )}
      </div>
      <h3>Overview</h3>
      <dl>
        {field('Cloud', 'cloud', r.cloud)}
        {field('Account', 'account', r.account)}
        {field('Region', 'region', r.region)}
        {field('Cluster', 'cluster', r.cluster)}
        {r.namespace && field('Namespace', 'namespace', r.namespace)}
        {field('Environment', 'env', r.env)}
        {field('Service', 'service', r.service)}
        {field('Team', 'team', r.team)}
        {r.createdAt && (
          <>
            <dt>Created</dt>
            <dd>{formatLongDate(r.createdAt)}</dd>
          </>
        )}
        <dt>Last seen</dt>
        <dd title={formatLongDate(r.lastSeen)}>{timeAgo(r.lastSeen)}</dd>
      </dl>
      {attributes.length > 0 && (
        <>
          <h3>Attributes</h3>
          <dl>
            {attributes.map(([k, v]) => (
              <Fragment key={k}>
                <dt>{ATTRIBUTE_LABELS[k] ?? k}</dt>
                <dd className="rc-wrap">{v}</dd>
              </Fragment>
            ))}
          </dl>
        </>
      )}
      <h3>Tags</h3>
      {r.tags.length ? (
        <div className="rc-tags">
          {r.tags.map((t) => {
            const i = t.indexOf(':');
            return (
              <button key={t} className="tag-chip" title="Filter by this tag" onClick={() => i > 0 && onFilter(t.slice(0, i), t.slice(i + 1))}>
                {t}
              </button>
            );
          })}
        </div>
      ) : (
        <p className="muted-text">No tags.</p>
      )}
    </aside>
  );
}
