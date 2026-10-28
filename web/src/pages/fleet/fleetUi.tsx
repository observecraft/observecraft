// Cells shared by the Fleet Automation views: agent type, OS, status, version and integration logos.
import { AlertTriangle, CheckCircle2, CircleSlash, Server } from 'lucide-react';
import type { FleetAgent, FleetAgentKind, FleetStatus } from '../../api/fleet/types';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { integrationLogo, kindLabel } from './fleetModel';

export function KindIcon({ kind, size = 16 }: { kind: FleetAgentKind; size?: number }) {
  // The Datadog Agent is named, not shown with Datadog's logo.
  return kind === 'otel_collector' ? <BrandLogo name="otel" size={size} title="OpenTelemetry" /> : <Server size={size} className="fl-dd-mark" aria-label="Datadog Agent" />;
}

export function KindCell({ kind }: { kind: FleetAgentKind }) {
  return (
    <span className="fl-cell-icon">
      <KindIcon kind={kind} />
      {kindLabel(kind)}
    </span>
  );
}

function osLogo(os: string | null | undefined, platform: string | null | undefined): string {
  const p = (platform ?? '').toLowerCase();
  if (p.includes('ubuntu')) return 'ubuntu';
  if (os === 'windows' || p.includes('windows')) return 'windows';
  if (os === 'darwin' || p.includes('mac')) return 'apple';
  return 'linux';
}

export function OsCell({ agent }: { agent: Pick<FleetAgent, 'os' | 'platform'> }) {
  if (!agent.os && !agent.platform) return <span className="muted">—</span>;
  const label = agent.platform ?? agent.os ?? '';
  return (
    <span className="fl-cell-icon" title={label}>
      <BrandLogo name={osLogo(agent.os, agent.platform)} size={15} title={agent.os ?? undefined} />
      <span className="fl-ellipsis">{label}</span>
    </span>
  );
}

const STATUS: Record<FleetStatus, { label: string; icon: typeof CheckCircle2 }> = {
  ok: { label: 'Reporting', icon: CheckCircle2 },
  issues: { label: 'Telemetry issues', icon: AlertTriangle },
  offline: { label: 'Not reporting', icon: CircleSlash },
};

export function StatusPill({ status }: { status: FleetStatus }) {
  const s = STATUS[status];
  const Icon = s.icon;
  return (
    <span className={`fl-status fl-status-${status}`}>
      <Icon size={13} /> {s.label}
    </span>
  );
}

export const statusLabel = (s: FleetStatus) => STATUS[s].label;

export function VersionCell({ agent }: { agent: Pick<FleetAgent, 'version' | 'outdated'> }) {
  if (!agent.version) return <span className="muted">unknown</span>;
  return (
    <span className={`fl-version${agent.outdated ? ' fl-version-outdated' : ''}`} title={agent.outdated ? 'An upgrade is available' : undefined}>
      {agent.version}
    </span>
  );
}

/** Logos of the running integrations, then "+N". */
export function IntegrationLogos({ integrations, max = 4 }: { integrations: string[]; max?: number }) {
  if (!integrations.length) return <span className="muted">—</span>;
  const shown = integrations.slice(0, max);
  return (
    <span className="fl-integrations">
      {shown.map((i) => (
        <span key={i} className="fl-integration" title={i}>
          <IntegrationLogo logo={integrationLogo(i)} name={i} size={14} />
        </span>
      ))}
      {integrations.length > max && (
        <span className="tag-chip" title={integrations.slice(max).join(', ')}>
          +{integrations.length - max}
        </span>
      )}
    </span>
  );
}
