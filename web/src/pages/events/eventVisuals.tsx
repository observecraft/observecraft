// Source icons and status badges of the event list.
import { Activity, Box, CalendarClock, GitCommitHorizontal, Server, Ship } from 'lucide-react';

/** A neutral mark per known source (no third-party logos). */
export function SourceIcon({ source, size = 18 }: { source: string; size?: number }) {
  const s = source.toLowerCase();
  const Icon = s.includes('kubernetes')
    ? Ship
    : s.includes('monitor') || s.includes('watchdog')
      ? Activity
      : s.includes('container') || s.includes('docker') || s.includes('ecs')
        ? Box
        : s.includes('change') || s.includes('deploy')
          ? GitCommitHorizontal
          : s === 'system' || s.includes('agent')
            ? Server
            : CalendarClock;
  return (
    <span className="ev-source-icon" title={source}>
      <Icon size={size} />
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
