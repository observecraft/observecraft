import { useEffect, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { BrandLogo } from '../../components/brand/BrandLogo';

export const SEVERITIES = ['critical', 'high', 'medium', 'low'] as const;

export function SeverityBadge({ severity, score }: { severity: string; score?: number | null }) {
  return (
    <span className="cs-sev-wrap">
      <span className={`cs-sev cs-sev-${severity}`}>{severity === 'unknown' ? 'NONE' : severity.toUpperCase()}</span>
      {score != null && <span className={`cs-score cs-sev-text-${severity}`}>{score.toFixed(1)}</span>}
    </span>
  );
}

const LANG_LOGO: Record<string, string> = { java: 'openjdk', nodejs: 'nodedotjs', python: 'python', dotnet: 'dotnet', go: 'go', ruby: 'ruby', php: 'php' };

export function LangLogo({ language, size = 14 }: { language: string; size?: number }) {
  const logo = LANG_LOGO[language];
  return logo ? <BrandLogo name={logo} size={size} title={language} /> : null;
}

/** A package with its version, as chips. */
export function PackageChip({ language, name, version }: { language: string; name: string; version?: string }) {
  return (
    <span className="cs-pkg">
      <LangLogo language={language} />
      <span className="cs-pkg-name">{name}</span>
      {version && <span className="cs-pkg-version">{version}</span>}
    </span>
  );
}

export function Pill({ tone = 'orange', icon, children }: { tone?: 'orange' | 'red' | 'blue' | 'gray' | 'green'; icon?: ReactNode; children: ReactNode }) {
  return (
    <span className={`cs-pill cs-pill-${tone}`}>
      {icon}
      {children}
    </span>
  );
}

/** Right-hand detail panel over the page; Esc closes it. */
export function Drawer({ onClose, children, wide }: { onClose: () => void; children: ReactNode; wide?: boolean }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);
  return (
    <div className="cs-drawer-backdrop" onClick={onClose}>
      <aside className={`cs-drawer${wide ? ' wide' : ''}`} onClick={(e) => e.stopPropagation()} role="dialog">
        <button className="icon-plain cs-drawer-close" onClick={onClose} aria-label="Close">
          <X size={18} />
        </button>
        {children}
      </aside>
    </div>
  );
}

/** Onboarding shown in place of a list that has never had data: what the product does and how to send it. */
export function Onboarding({ icon, product, title, text, children }: { icon: ReactNode; product: string; title: string; text: string; children?: ReactNode }) {
  return (
    <div className="cs-hero">
      <div className="cs-hero-text">
        <span className="cs-hero-product">
          <span className="cs-hero-icon">{icon}</span> {product}
        </span>
        <h2>{title}</h2>
        <p>{text}</p>
        {children}
      </div>
    </div>
  );
}

export function timeAgo(t: number | null | undefined): string {
  if (!t) return '—';
  const s = Math.max(0, (Date.now() - t) / 1000);
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  if (s < 86_400) return `${Math.round(s / 3600)}h ago`;
  if (s < 86_400 * 60) return `${Math.round(s / 86_400)}d ago`;
  return `${Math.round(s / (86_400 * 30))}mo ago`;
}

export const fmtDate = (t: number | null | undefined) => (t ? new Date(t).toLocaleString('en-US', { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' }) : '—');

export const shortRepo = (r: string) => r.replace(/^[^/]+\//, '');
