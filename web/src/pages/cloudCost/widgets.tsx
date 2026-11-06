// Building blocks shared by the Cloud Cost pages.
import type { ReactNode } from 'react';
import { ArrowDownRight, ArrowUpRight } from 'lucide-react';
import type { CostTag } from '../../api/cloudCost/client';
import { INTEGRATION_LOGOS } from '../../assets/integrationLogos.generated';
import { BRAND_LOGOS } from '../../assets/brandLogos.generated';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';
import { escapeHtml } from '../../lib/html';
import { Select } from '../../components/ui/Dropdown';
import { CloudIcon } from '../infrastructure/hostIcons';
import { money, pctLabel, providerColor, providerLabel, tagValues, type Filters } from './costModel';

/** Logo of a provider or AI vendor whose name isn't its logo's key; others match by name ("Snowflake" → snowflake). */
const LOGOS: Record<string, string> = { 'amazon bedrock': 'aws', 'azure ai foundry': 'azure', 'vertex ai': 'gcp' };

const logoKey = (name: string) => LOGOS[name.toLowerCase()] ?? name.toLowerCase().replace(/[^a-z0-9]/g, '');

/**
 * A provider's logo, or its product's (`service`, e.g. AmazonEC2) when there is one; the provider's
 * initial in a colored square when there is no logo for it.
 */
export function ProviderIcon({ provider, service, size = 18 }: { provider: string; service?: string; size?: number }) {
  const label = service ? `${providerLabel(provider)} ${service}` : providerLabel(provider);
  if (service && hasBrandLogo(logoKey(service))) {
    return (
      <span className="ccm-provider" title={label} style={{ width: size, height: size }}>
        <BrandLogo name={logoKey(service)} size={size} title={label} />
      </span>
    );
  }
  if (provider === 'oci') return <CloudIcon cloud="oracle" size={size} />;
  const key = logoKey(provider);
  if (hasBrandLogo(key)) {
    return (
      <span className="ccm-provider" title={label} style={{ width: size, height: size }}>
        <BrandLogo name={key} size={size} title={label} />
      </span>
    );
  }
  return (
    <span className="ccm-provider ccm-provider-initial" title={label} style={{ width: size, height: size, background: providerColor(provider), fontSize: size * 0.55 }}>
      {provider.charAt(0).toUpperCase()}
    </span>
  );
}

/** Google Cloud's billing service names whose service icon isn't named after them. */
const GCP_SERVICES: Record<string, string> = {
  'compute engine': 'gce',
  'cloud storage': 'gcs',
  'kubernetes engine': 'gke',
  'cloud sql': 'cloudsql',
  'cloud run': 'run',
  'cloud functions': 'cloudfunctions',
  'cloud pub/sub': 'pubsub',
  'cloud memorystore for redis': 'redis',
  'cloud dns': 'dns',
  'vertex ai': 'aiplatform',
};

/** The cloud a billing service name belongs to (`AmazonEC2`, `Microsoft.Compute`, `Compute Engine`), if it's recognisable. */
function serviceCloud(service: string): 'aws' | 'azure' | 'gcp' | null {
  if (/^(amazon|aws)/i.test(service)) return 'aws';
  if (/^(microsoft\.|azure|virtualmachines$|foundry)/i.test(service)) return 'azure';
  if (GCP_SERVICES[service.toLowerCase()] || /^(cloud |google|bigquery)/i.test(service)) return 'gcp';
  return null;
}

/** The cloud provider's own icon of a billing service, from the integrations catalog's logo files. */
function serviceTile(service: string): string | null {
  const cloud = serviceCloud(service);
  const has = (k: string) => k in INTEGRATION_LOGOS;
  if (cloud === 'aws') {
    const key = `tile:aws-${service.replace(/^(amazon|aws)/i, '').toLowerCase()}`;
    return has(key) ? key : null;
  }
  if (cloud === 'azure') {
    // Microsoft.Compute → the resource provider's first resource type (tile:azure-compute-…), shortest first.
    const ns = service.toLowerCase() === 'virtualmachines' ? 'compute' : service.replace(/^microsoft\./i, '').toLowerCase();
    const keys = Object.keys(INTEGRATION_LOGOS)
      .filter((k) => k.startsWith(`tile:azure-${ns}-`) || k === `tile:azure-${ns}`)
      .sort((a, b) => a.length - b.length);
    return keys.find((k) => k.endsWith('virtualmachines')) ?? keys[0] ?? null;
  }
  if (cloud === 'gcp') {
    const key = `tile:gcp-${
      GCP_SERVICES[service.toLowerCase()] ??
      service
        .toLowerCase()
        .replace(/^cloud /, '')
        .replace(/[^a-z0-9]/g, '')
    }`;
    return has(key) ? key : null;
  }
  return null;
}

/** A billing service's icon: its product logo, the cloud's service icon, or its provider's logo; none when unknown. */
export function ServiceIcon({ service, size = 16 }: { service: string; size?: number }) {
  if (hasBrandLogo(logoKey(service))) {
    return (
      <span className="ccm-provider" title={service} style={{ width: size, height: size }}>
        <BrandLogo name={logoKey(service)} size={size} title={service} />
      </span>
    );
  }
  const tile = serviceTile(service);
  if (tile) {
    return (
      <span className="ccm-provider" title={service} style={{ width: size, height: size }}>
        <img className="logo-file" src={`/images/integrations/logos/${INTEGRATION_LOGOS[tile]}`} alt="" loading="lazy" width={size} height={size} />
      </span>
    );
  }
  const cloud = serviceCloud(service);
  return cloud ? <ProviderIcon provider={cloud} size={size} /> : null;
}

/** The icon of a cost tag's value in pickers and filters: providers and services show their logos. */
export function costValueIcon(key: string, value: string, size = 16): ReactNode {
  if (!value) return undefined;
  if (key === 'providername') return <ProviderIcon provider={value} size={size} />;
  if (key === 'servicename') return <ServiceIcon service={value} size={size} />;
  return undefined;
}

// ---------------------------------------------------------------- chart tooltips (HTML strings)

const box = (size: number) => `display:inline-flex;width:${size}px;height:${size}px;margin-right:6px;vertical-align:-3px;flex:none`;

function brandHtml(name: string, size: number): string {
  const logo = BRAND_LOGOS[name];
  return `<svg viewBox="0 0 ${logo.width} ${logo.height}" width="${size}" height="${size}" style="${box(size)}" aria-hidden="true">${logo.body}</svg>`;
}

function providerHtml(provider: string, size: number): string {
  const key = logoKey(provider);
  if (hasBrandLogo(key)) return brandHtml(key, size);
  return `<span style="${box(size)};align-items:center;justify-content:center;border-radius:3px;background:${providerColor(provider)};color:#fff;font-size:${Math.round(size * 0.6)}px;font-weight:600">${escapeHtml(provider.charAt(0).toUpperCase())}</span>`;
}

/**
 * A cost series' icon in a chart tooltip, from its tags (the same choice as [`costValueIcon`]): its service's icon
 * when it is grouped by service, else its provider's; empty for other groupings.
 */
export function costIconHtml(tags: Record<string, string> | undefined, size = 14): string {
  const service = tags?.servicename;
  if (service) {
    if (hasBrandLogo(logoKey(service))) return brandHtml(logoKey(service), size);
    const tile = serviceTile(service);
    if (tile) return `<img src="/images/integrations/logos/${INTEGRATION_LOGOS[tile]}" alt="" width="${size}" height="${size}" style="${box(size)};object-fit:contain">`;
    const cloud = serviceCloud(service);
    if (cloud) return providerHtml(cloud, size);
  }
  const provider = tags?.providername;
  return provider && provider !== 'oci' ? providerHtml(provider, size) : '';
}

/**
 * An axis tooltip for stacked cost charts: the bucket, then each series with its marker, its service or provider icon
 * (`tagsOf` gives a series' tags by its name), its name and its cost; series without cost in the bucket are left out.
 */
export function costAxisTooltip(tagsOf: (seriesName: string) => Record<string, string> | undefined) {
  return {
    trigger: 'axis' as const,
    axisPointer: { type: 'shadow' as const },
    confine: true,
    formatter: (params: unknown) => {
      const list = (Array.isArray(params) ? params : [params]) as { axisValueLabel?: string; marker?: string; seriesName?: string; value?: unknown }[];
      const rows = list
        .filter((p) => p.value != null && p.value !== '' && Number(p.value) !== 0)
        .map(
          (p) =>
            `<div style="display:flex;align-items:center;gap:2px;line-height:22px">${p.marker ?? ''}${costIconHtml(tagsOf(p.seriesName ?? ''))}<span style="flex:1;margin-right:16px">${escapeHtml(p.seriesName ?? '')}</span><b>${money(Number(p.value), { precise: true })}</b></div>`,
        );
      return `<div style="margin-bottom:2px">${escapeHtml(list[0]?.axisValueLabel ?? '')}</div>${rows.join('')}`;
    },
  };
}

/** `22% ↗` in red for increases, green for decreases (lower cost is good). */
export function ChangePill({ pct, digits = 0 }: { pct: number | null; digits?: number }) {
  if (pct == null) return <span className="ccm-pill ccm-pill-up">new</span>;
  const up = pct > 0;
  return (
    <span className={`ccm-pill ${up ? 'ccm-pill-up' : pct < 0 ? 'ccm-pill-down' : ''}`}>
      {pctLabel(pct, digits).replace('+', '')} {up ? <ArrowUpRight size={13} /> : pct < 0 ? <ArrowDownRight size={13} /> : null}
    </span>
  );
}

/** The round arrow in front of a headline. */
export function TrendIcon({ up }: { up: boolean | null }) {
  if (up == null) return <span className="ccm-trend" />;
  return <span className={`ccm-trend ${up ? 'ccm-trend-up' : 'ccm-trend-down'}`}>{up ? <ArrowUpRight size={18} /> : <ArrowDownRight size={18} />}</span>;
}

/** "Filter to" dropdowns over cost tags; each picks one value or All. */
export function CostFilterBar({
  keys,
  labels,
  filters,
  onChange,
  tags,
  prefix = 'Filter to',
  children,
}: {
  keys: string[];
  labels?: Record<string, string>;
  filters: Filters;
  onChange: (f: Filters) => void;
  tags: CostTag[] | undefined;
  prefix?: string;
  children?: ReactNode;
}) {
  return (
    <div className="ccm-filterbar">
      {prefix && <span className="ccm-filterbar-label">{prefix}</span>}
      {keys.map((k) => (
        <Select
          key={k}
          label={labels?.[k] ?? k}
          value={filters[k]?.[0] ?? ''}
          options={[{ value: '', label: 'All' }, ...tagValues(tags, k).map((v) => ({ value: v, label: k === 'providername' ? providerLabel(v) : v, icon: costValueIcon(k, v) }))]}
          onChange={(v) => onChange({ ...filters, [k]: v ? [v] : [] })}
          triggerClassName="dd-trigger ccm-filter"
        />
      ))}
      {children}
    </div>
  );
}

/** A horizontal bar: the share of the largest value. */
export function ShareBar({ share, tone = 'blue' }: { share: number; tone?: 'blue' | 'red' | 'green' }) {
  return (
    <span className={`ccm-sharebar ccm-sharebar-${tone}`}>
      <span style={{ width: `${Math.max(0, Math.min(1, share)) * 100}%` }} />
    </span>
  );
}

export function SectionTitle({ children, right }: { children: ReactNode; right?: ReactNode }) {
  return (
    <div className="ccm-section-title">
      <h2>{children}</h2>
      {right}
    </div>
  );
}
