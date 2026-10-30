// Source icons and status badges of the event list.
import { Activity, CalendarClock, GitCommitHorizontal, Server } from 'lucide-react';
import { BrandLogo } from '../../components/brand/BrandLogo';

/** The platform's logo for platform sources (Kubernetes, Docker, ECS), else a neutral mark per kind of source. */
export function SourceIcon({ source, size = 18 }: { source: string; size?: number }) {
  const s = source.toLowerCase();
  const logo = s.includes('kubernetes') ? 'kubernetes' : s.includes('ecs') ? 'amazonecs' : s.includes('docker') || s.includes('container') ? 'docker' : null;
  const Icon =
    s.includes('monitor') || s.includes('watchdog') ? Activity : s.includes('change') || s.includes('deploy') ? GitCommitHorizontal : s === 'system' || s.includes('agent') ? Server : CalendarClock;
  return (
    <span className="ev-source-icon" title={source}>
      {logo ? <BrandLogo name={logo} size={size} title={source} /> : <Icon size={size} />}
    </span>
  );
}

const LABEL: Record<string, string> = { error: 'ERROR', warning: 'WARN', info: 'INFO', success: 'OK' };

export function StatusBadge({ status }: { status: string }) {
  return <span className={`ev-status ev-status-${status}`}>{LABEL[status] ?? status.toUpperCase()}</span>;
}

/** Bar colour per event status, for the histogram. */
export const STATUS_TOKENS: Record<string, string> = {
  error: '--status-error',
  warning: '--status-warn',
  info: '--status-info',
  success: '--status-ok',
};
