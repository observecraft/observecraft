import { ExternalLink, GitBranch, GitCommitHorizontal } from 'lucide-react';
import type { CiFilters } from '../../api/ci/client';

export const NO_FILTERS: CiFilters = {
  statuses: [],
  pipelines: [],
  repositories: [],
  branches: [],
  providers: [],
  teams: [],
};

export const STATUS_COLOR: Record<string, string> = {
  success: 'var(--ci-success)',
  error: 'var(--ci-error)',
  canceled: 'var(--ci-canceled)',
  skipped: 'var(--ci-canceled)',
};
export const STATUS_LABEL: Record<string, string> = {
  success: 'Success',
  error: 'Error',
  canceled: 'Canceled',
  skipped: 'Skipped',
};
export const PROVIDER_LABEL: Record<string, string> = {
  github_actions: 'GitHub Actions',
  gitlab: 'GitLab',
  jenkins: 'Jenkins',
  circleci: 'CircleCI',
  buildkite: 'Buildkite',
  azure_pipelines: 'Azure Pipelines',
};

/** "1m 13s", "16.0s", "2h 5m". */
export function fmtDur(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return '—';
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${Math.round(s % 60)}s`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ${m % 60}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

export function fmtDate(ms: number): string {
  const d = new Date(ms);
  return `${d.toLocaleString(undefined, { month: 'short', day: 'numeric' })} ${d.toLocaleTimeString(undefined, { hour12: false })}`;
}

export function fmtAgo(ms: number): string {
  const s = (Date.now() - ms) / 1000;
  if (s < 90) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

export function StatusPill({ status }: { status: string }) {
  return <span className={`ci-pill ci-${status}`}>{STATUS_LABEL[status] ?? status}</span>;
}

export function Branch({ name }: { name: string }) {
  return (
    <span className="ci-branch">
      <GitBranch size={14} /> {name}
    </span>
  );
}

export function Sha({ sha }: { sha: string }) {
  return (
    <span className="ci-sha" title={sha}>
      <GitCommitHorizontal size={14} /> {sha.slice(0, 7)}
    </span>
  );
}

export function Ext({ href, children }: { href: string; children: React.ReactNode }) {
  if (!href) return <>{children}</>;
  return (
    <a href={href} target="_blank" rel="noreferrer noopener" className="ci-ext" onClick={(e) => e.stopPropagation()}>
      {children} <ExternalLink size={13} />
    </a>
  );
}
