// Building blocks shared by the Cloud Cost pages.
import type { ReactNode } from 'react';
import { ArrowDownRight, ArrowUpRight } from 'lucide-react';
import type { CostTag } from '../../api/cloudCost/client';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';
import { Select } from '../../components/ui/Dropdown';
import { CloudIcon } from '../infrastructure/hostIcons';
import { pctLabel, providerColor, providerLabel, tagValues, type Filters } from './costModel';

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
          options={[{ value: '', label: 'All' }, ...tagValues(tags, k).map((v) => ({ value: v, label: k === 'providername' ? providerLabel(v) : v }))]}
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
