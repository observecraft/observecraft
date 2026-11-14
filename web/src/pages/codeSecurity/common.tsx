import { useEffect, type ReactNode } from 'react';
import { ExternalLink, GitBranch, X } from 'lucide-react';
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

/** A repository's source control, from its host (the server also knows self-managed GitLab hosts: pass `provider`). */
export function repoProvider(repository: string): string {
  const host = repository.split('/')[0].toLowerCase();
  if (!repository.includes('/')) return 'git';
  if (host === 'github.com' || host.startsWith('github.')) return 'github';
  if (host === 'gitlab.com' || host.startsWith('gitlab.')) return 'gitlab';
  return host.includes('bitbucket') ? 'bitbucket' : 'git';
}

const PROVIDER_LABEL: Record<string, string> = { github: 'GitHub', gitlab: 'GitLab', bitbucket: 'Bitbucket', git: 'Git' };

/** A repository with its source control's logo; a link to it with `link`. */
export function RepoName({ repository, provider, link }: { repository: string; provider?: string; link?: boolean }) {
  const p = provider || repoProvider(repository);
  const icon = p === 'git' ? <GitBranch size={14} /> : <BrandLogo name={p} size={14} title={PROVIDER_LABEL[p]} />;
  const name = (
    <>
      {icon} {shortRepo(repository)}
    </>
  );
  if (!link || !repository.includes('/')) return <span className="cs-repo">{name}</span>;
  return (
    <a className="cs-repo" href={`https://${repository}`} target="_blank" rel="noreferrer" onClick={(e) => e.stopPropagation()} title={`Open ${repository} in ${PROVIDER_LABEL[p]}`}>
      {name}
    </a>
  );
}

/** Scanner name → logo key (only scanners with a logo in the app's sets; the others show their name alone). */
const SCANNER_LOGO: Record<string, string> = { 'datadog-static-analyzer': 'datadog', trivy: 'trivy', snyk: 'snyk', checkmarx: 'checkmarx', sonarqube: 'sonarqube', codeql: 'github' };

export function ScannerName({ tool }: { tool: string }) {
  const logo = SCANNER_LOGO[tool.toLowerCase()];
  return (
    <span className="cs-scanner muted">
      {logo && <BrandLogo name={logo} size={13} title={tool} />} {tool}
    </span>
  );
}

const SOURCE_LABEL: Record<string, string> = { gitlab: 'GitLab', upload: 'CI upload', runtime: 'Runtime' };

/** Where a finding was reported: GitLab's Vulnerability Report (linked), a CI upload, the tracers. */
export function SourceBadges({ sources, url }: { sources: string[]; url?: string }) {
  return (
    <span className="cs-sources">
      {sources.map((s) =>
        s === 'gitlab' && url ? (
          <a key={s} className="cs-chip cs-source" href={url} target="_blank" rel="noreferrer" onClick={(e) => e.stopPropagation()} title="Open in GitLab's Vulnerability Report">
            <BrandLogo name="gitlab" size={12} /> GitLab <ExternalLink size={11} />
          </a>
        ) : (
          <span key={s} className="cs-chip cs-source">
            {s === 'gitlab' && <BrandLogo name="gitlab" size={12} />} {SOURCE_LABEL[s] ?? s}
          </span>
        ),
      )}
    </span>
  );
}
