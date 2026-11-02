import { BRAND_LOGOS } from '../../assets/brandLogos.generated';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { Select } from '../../components/ui/Dropdown';

/** Shared labels and marks for network grouping controls and graph nodes. */
export const NETWORK_GROUPS = [
  { key: 'service', label: 'Service', path: 'M12 3v6m-7 6v6m14-6v6M5 15V9h14v6M9 3h6v6H9zM2 15h6v6H2zM16 15h6v6h-6z' },
  { key: 'env', label: 'Environment', path: 'm12 3 9 5-9 5-9-5 9-5ZM3 12l9 5 9-5M3 16l9 5 9-5' },
  { key: 'availability-zone', label: 'Availability Zone', path: 'M4 3h16v6H4zM4 15h16v6H4zM8 6h.01M8 18h.01M12 9v6' },
  { key: 'kube_namespace', label: 'Kubernetes Namespace', brand: 'kubernetes' },
  { key: 'kube_service', label: 'Kubernetes Service', brand: 'kubernetes' },
  { key: 'pod_name', label: 'Pod', brand: 'kubernetes' },
  { key: 'container_name', label: 'Container', brand: 'docker' },
  { key: 'short_image', label: 'Container Image', brand: 'docker' },
  { key: 'host', label: 'Host', path: 'M3 3h18v7H3zM3 14h18v7H3zM7 6.5h.01M7 17.5h.01' },
  { key: 'ip', label: 'IP', path: 'M9 3h6v6H9zM2 15h6v6H2zM16 15h6v6h-6zM12 9v3M5 15v-3h14v3' },
  { key: 'port', label: 'Port', path: 'M5 3h14v11l-4 7H9l-4-7V3ZM9 3v5m6-5v5M9 17h6' },
  { key: 'domain', label: 'Domain', path: 'M3 12h18M12 3c-6 5-6 13 0 18 6-5 6-13 0-18Z', circle: true },
] as const;

export function networkGroupMark(key: string): { body: string; width: number; height: number } {
  const group = NETWORK_GROUPS.find((g) => g.key === (key === 'dns_server' ? 'host' : key === 'dns_query' ? 'domain' : key)) ?? NETWORK_GROUPS[0];
  if ('brand' in group) return BRAND_LOGOS[group.brand];
  return {
    width: 24,
    height: 24,
    body: `<g fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${'circle' in group ? '<circle cx="12" cy="12" r="9"/>' : ''}<path d="${group.path}"/></g>`,
  };
}

function GroupIcon({ groupKey }: { groupKey: string }) {
  const group = NETWORK_GROUPS.find((g) => g.key === groupKey);
  if (group && 'brand' in group) return <BrandLogo name={group.brand} size={16} />;
  const mark = networkGroupMark(groupKey);
  return <svg width={16} height={16} viewBox="0 0 24 24" aria-hidden="true" dangerouslySetInnerHTML={{ __html: mark.body }} />;
}

export function NetworkGroupSelect({
  value,
  onChange,
  label,
  dns = false,
  groups = NETWORK_GROUPS,
}: {
  value: string;
  onChange: (value: string) => void;
  label: string;
  dns?: boolean;
  groups?: readonly { key: string; label: string }[];
}) {
  return (
    <div className="net-chipset net-group-select">
      <span>{label}</span>
      <Select
        title={`Group by ${label.toLowerCase()}`}
        value={value}
        options={groups.filter((g) => !dns || (g.key !== 'port' && g.key !== 'domain')).map((g) => ({ value: g.key, label: g.label, icon: <GroupIcon groupKey={g.key} /> }))}
        onChange={onChange}
      />
    </div>
  );
}
