import type { ReactNode } from 'react';
import { AlertCircle, Database, Globe, RefreshCcw, Smartphone, User as UserIcon } from 'lucide-react';
import { BrandLogo } from '../../components/brand/BrandLogo';
import type { Issue, IssueStatus, Platform, SuspectedCause, User } from '../../api/errorTracking/types';

export const STATUS_LABEL: Record<IssueStatus, string> = {
  for_review: 'For Review',
  reviewed: 'Reviewed',
  ignored: 'Ignored',
  resolved: 'Resolved',
};

export const CAUSE_LABEL: Record<SuspectedCause, string> = {
  code_exception: 'Code exception',
  failed_request: 'Failed request',
  lost_connection: 'Lost connection',
  invalid_argument: 'Invalid argument',
  illegal_object_access: 'Illegal object access',
  unknown: 'Unknown',
};

const PLATFORM_LOGO: Record<Platform, { logo: string; label: string }> = {
  android: { logo: 'android', label: 'Android' },
  java: { logo: 'openjdk', label: 'Java' },
  dotnet: { logo: 'dotnet', label: '.NET' },
  browser: { logo: 'javascript', label: 'JavaScript' },
  'react-native': { logo: 'react', label: 'React Native' },
  node: { logo: 'nodedotjs', label: 'Node.js' },
  python: { logo: 'python', label: 'Python' },
  go: { logo: 'go', label: 'Go' },
  ios: { logo: 'apple', label: 'iOS' },
};

/** Runtime logo before the error type (full-colour brand marks). */
export function PlatformIcon({ platform }: { platform: Platform }) {
  const p = PLATFORM_LOGO[platform];
  return (
    <span className="platform-icon">
      <BrandLogo name={p.logo} size={16} title={p.label} />
    </span>
  );
}

/** Application / service chip: icon tile coloured by where the error comes from. */
export function AppChip({ issue, withName = false }: { issue: Pick<Issue, 'persona' | 'source' | 'service' | 'application'>; withName?: boolean }) {
  let icon: ReactNode;
  let tone: string;
  if (issue.persona === 'mobile') {
    icon = <Smartphone size={13} />;
    tone = 'mobile';
  } else if (issue.persona === 'browser') {
    icon = <Globe size={13} />;
    tone = 'browser';
  } else if (issue.source === 'logs') {
    icon = <Database size={13} />;
    tone = 'logs';
  } else {
    icon = <Globe size={13} />;
    tone = 'apm';
  }
  const label = issue.application?.packageId ?? issue.application?.name ?? issue.service;
  const suffix = withName && issue.application?.packageId ? ` (${issue.application.name})` : '';
  return (
    <span className="app-chip">
      <span className={`app-chip-icon tone-${tone}`}>{icon}</span>
      {label}
      {suffix}
    </span>
  );
}

export function CrashBadge() {
  return (
    <span className="et-badge et-badge-crash">
      <AlertCircle size={13} fill="currentColor" stroke="var(--status-error)" /> Crash
    </span>
  );
}

export function RegressionBadge() {
  return (
    <span className="et-badge et-badge-regression">
      <RefreshCcw size={12} /> Regression
    </span>
  );
}

/** 12-bucket occurrence bars. Inline SVG: dozens per page, so no chart library. */
export function Sparkline({ values, width = 114, height = 34 }: { values: number[]; width?: number; height?: number }) {
  const max = Math.max(1, ...values);
  const bw = width / values.length;
  return (
    <svg width={width} height={height} className="sparkline" aria-hidden>
      {values.map((v, i) => {
        const h = v === 0 ? 0 : Math.max(2, (v / max) * (height - 2));
        return <rect key={i} x={i * bw + 1} y={height - 1 - h} width={bw - 2} height={h} fill="var(--status-error)" />;
      })}
      <line x1="0" x2={width} y1={height - 0.5} y2={height - 0.5} stroke="var(--border-strong)" />
    </svg>
  );
}

const AVATAR_COLORS = ['#0b6e69', '#2f6ad0', '#29a8a0', '#d65cad', '#f2a93b', '#5fb85a'];

export function Avatar({ user, size = 26 }: { user: User | null; size?: number }) {
  if (!user)
    return (
      <span className="avatar-empty" style={{ width: size, height: size }}>
        <UserIcon size={size * 0.6} />
      </span>
    );
  const initials = user.name
    .split(' ')
    .map((p) => p[0])
    .join('')
    .slice(0, 2);
  const color = AVATAR_COLORS[[...user.id].reduce((a, c) => a + c.charCodeAt(0), 0) % AVATAR_COLORS.length];
  return (
    <span className="avatar-initials" style={{ width: size, height: size, background: color, fontSize: size * 0.42 }} title={user.name}>
      {initials}
    </span>
  );
}

/** "99+" style tab counts. */
export function capCount(n: number): string {
  return n > 99 ? '99+' : String(n);
}
