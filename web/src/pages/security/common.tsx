import type { ReactNode } from 'react';
import { NavLink, useNavigate } from 'react-router-dom';
import { Boxes, ChevronDown, Settings, ShieldCheck } from 'lucide-react';
import { Menu } from '../../components/ui/Dropdown';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';

export const SEVERITY_LABEL: Record<string, string> = { critical: 'Critical', high: 'High', medium: 'Medium', low: 'Low', info: 'Info' };
export const STATE_LABEL: Record<string, string> = { open: 'Open', under_review: 'Under Review', archived: 'Archived' };
export const ARCHIVE_REASONS: [string, string][] = [
  ['false_positive', 'False positive'],
  ['testing_or_maintenance', 'Testing or maintenance'],
  ['true_positive_benign', 'True positive: benign'],
  ['true_positive_malicious', 'True positive: malicious'],
  ['investigated_case_opened', 'Investigated, case opened'],
  ['other', 'Other'],
];

export function SeverityBadge({ severity }: { severity: string }) {
  return <span className={`sec-sev sev-${severity}`}>{(SEVERITY_LABEL[severity] ?? severity).toUpperCase()}</span>;
}

export function SeverityDot({ severity }: { severity: string }) {
  return <i className={`sec-dot sev-${severity}`} aria-hidden />;
}

export function PackLogo({ logo, size = 28 }: { logo: string; size?: number }) {
  return hasBrandLogo(logo) ? <BrandLogo name={logo} size={size} /> : <Boxes size={size} />;
}

/** Product header shared by Security and Cloud SIEM: the product switcher, its tabs, and Settings. */
export function SecurityHeader({ product, tabs, settings }: { product: 'Security' | 'Cloud SIEM'; tabs: [string, string, boolean?][]; settings: string }) {
  const navigate = useNavigate();
  return (
    <header className="product-header sec-header">
      <Menu
        align="left"
        triggerClassName="product-title sec-switch"
        trigger={
          <>
            <ShieldCheck size={20} /> {product} <ChevronDown size={16} />
          </>
        }
        items={[
          { label: 'Security', onClick: () => navigate('/security') },
          { label: 'Cloud SIEM', onClick: () => navigate('/security/siem') },
          { label: 'Cloud Security', onClick: () => navigate('/security/cloud') },
          { label: 'App & API Protection', onClick: () => navigate('/security/appsec') },
          { label: 'Code Security', onClick: () => navigate('/code-security') },
        ]}
      />
      <nav className="product-tabs">
        {tabs.map(([label, to, end]) => (
          <NavLink key={to} to={to} end={end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            {label}
          </NavLink>
        ))}
      </nav>
      <div className="mon-header-actions">
        <button className="btn" onClick={() => navigate(settings)}>
          <Settings size={15} /> Settings
        </button>
      </div>
    </header>
  );
}

export function Stat({ value, label, to }: { value: ReactNode; label: string; to?: string }) {
  const navigate = useNavigate();
  return (
    <button className={`sec-stat${to ? ' link' : ''}`} onClick={() => to && navigate(to)} disabled={!to}>
      <b>{value}</b>
      <span>{label}</span>
    </button>
  );
}
