import type { ReactNode } from 'react';
import {
  AppWindow,
  Bell,
  Braces,
  CalendarClock,
  Clock,
  Gauge,
  GitBranch,
  Globe,
  Hand,
  Megaphone,
  MessageSquare,
  Repeat,
  ScrollText,
  Siren,
  Timer,
  TriangleAlert,
  Webhook,
  Workflow as WorkflowIcon,
  Zap,
} from 'lucide-react';
import type { WorkflowRunStatus, WorkflowStepKind, WorkflowTriggerKind } from '../../../api/workflows/types';
import { IntegrationLogo } from '../../integrations/IntegrationLogo';

/** Icon and tile colour of each step kind (colours are theme tokens). */
export const STEP_ICON: Record<WorkflowStepKind, { icon: ReactNode; tone: string }> = {
  http: { icon: <Globe size={16} />, tone: 'blue' },
  create_incident: { icon: <Siren size={16} />, tone: 'red' },
  page_on_call: { icon: <Megaphone size={16} />, tone: 'red' },
  incident_comment: { icon: <MessageSquare size={16} />, tone: 'red' },
  synthetic_run: { icon: <Gauge size={16} />, tone: 'green' },
  notify: { icon: <Bell size={16} />, tone: 'orange' },
  metric_query: { icon: <Gauge size={16} />, tone: 'purple' },
  log_count: { icon: <ScrollText size={16} />, tone: 'purple' },
  wait: { icon: <Timer size={16} />, tone: 'gray' },
  if: { icon: <GitBranch size={16} />, tone: 'teal' },
  for_each: { icon: <Repeat size={16} />, tone: 'teal' },
};

export const TRIGGER_ICON: Record<WorkflowTriggerKind, ReactNode> = {
  manual: <Hand size={14} />,
  monitor: <TriangleAlert size={14} />,
  schedule: <CalendarClock size={14} />,
  incident: <Siren size={14} />,
  api: <Webhook size={14} />,
  app: <AppWindow size={14} />,
};

export function StepTile({ kind, size = 32 }: { kind: WorkflowStepKind; size?: number }) {
  const s = STEP_ICON[kind];
  return (
    <span className={`wf-tile wf-tone-${s.tone}`} style={{ width: size, height: size }}>
      {s.icon}
    </span>
  );
}

/** A tool logo chip: a registry logo, or one of our own icons for internal sources. */
export function LogoChip({ logo, size = 28 }: { logo: string; size?: number }) {
  const inner = size * 0.6;
  const own: Record<string, ReactNode> = {
    monitor: <TriangleAlert size={inner} />,
    schedule: <Clock size={inner} />,
    incident: <Siren size={inner} />,
    internal: <Zap size={inner} />,
    workflow: <WorkflowIcon size={inner} />,
    expression: <Braces size={inner} />,
  };
  return (
    <span className={`wf-logo-chip${own[logo] ? ' wf-logo-own' : ''}`} style={{ width: size, height: size }}>
      {own[logo] ?? <IntegrationLogo logo={logo} name={logo} size={inner} />}
    </span>
  );
}

const STATUS_LABEL: Record<WorkflowRunStatus, string> = { queued: 'Queued', running: 'Running', waiting: 'Waiting', succeeded: 'Succeeded', failed: 'Failed', canceled: 'Canceled' };

export function RunStatusChip({ status }: { status: string | null | undefined }) {
  if (!status) return <span className="wf-muted">—</span>;
  return <span className={`wf-status wf-status-${status}`}>{STATUS_LABEL[status as WorkflowRunStatus] ?? status}</span>;
}

export function fmtAgo(ms: number | null | undefined, now = Date.now()): string {
  if (!ms) return '—';
  const d = Math.max(0, now - ms);
  if (d < 60_000) return 'just now';
  if (d < 3_600_000) return `${Math.floor(d / 60_000)} min ago`;
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)} h ago`;
  const days = Math.floor(d / 86_400_000);
  return days === 1 ? 'yesterday' : `${days} days ago`;
}

export function fmtDate(ms: number | null | undefined): string {
  if (!ms) return '—';
  return new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

export function fmtDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return '—';
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.floor(ms / 60_000)} min ${Math.round((ms % 60_000) / 1000)} s`;
}
