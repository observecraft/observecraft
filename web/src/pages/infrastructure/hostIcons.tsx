// Marks for clouds, OSes, collectors and software in the host list and filters.
// Full-colour brand logos via BrandLogo (SVG Logos, CC0); Alibaba uses its Simple Icons mark in brand colour.
import { Cpu, Server } from 'lucide-react';
import { siAlibabacloud, siOracle, type SimpleIcon } from 'simple-icons';
import type { Cloud, CollectorKind, HostOs, Software } from '../../api/infrastructure/types';
import { BrandLogo } from '../../components/brand/BrandLogo';

/** ObserveCraft collector (OTel Collector + Vector bundle) — our own brand glyph. */
function CollectorMark({ size = 16 }: { size?: number }) {
  return (
    <svg viewBox="0 0 16 16" width={size} height={size} role="img" aria-label="ObserveCraft Collector">
      <title>ObserveCraft Collector</title>
      <rect width="16" height="16" rx="4" fill="var(--brand)" />
      <path d="M3.5 10.5 6.5 7.5 8.5 9.5 12.5 5" fill="none" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function SiMark({ icon, size, title }: { icon: SimpleIcon; size: number; title: string }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} fill={`#${icon.hex}`} role="img" aria-label={title}>
      <path d={icon.path} />
    </svg>
  );
}

export const CLOUD_LABEL: Record<Cloud, string> = { aws: 'AWS', azure: 'Azure', gcp: 'Google Cloud', oracle: 'Oracle', alibaba: 'Alibaba Cloud', none: 'On-prem' };
export const OS_LABEL: Record<HostOs, string> = { windows: 'Windows', linux: 'Linux', darwin: 'Darwin' };
export const COLLECTOR_LABEL: Record<CollectorKind, string> = { observecraft: 'ObserveCraft Agent', datadog: 'Datadog Agent', otel: 'OpenTelemetry' };

export function CloudIcon({ cloud, size = 16 }: { cloud: Cloud; size?: number }) {
  switch (cloud) {
    case 'aws':
      return <BrandLogo name="aws" size={size} title="AWS" />;
    case 'azure':
      return <BrandLogo name="azure" size={size} title="Azure" />;
    case 'gcp':
      return <BrandLogo name="gcp" size={size} title="Google Cloud" />;
    case 'oracle':
      // The full-colour Oracle logo is a wordmark, illegible at icon size; use the brand ring mark.
      return <SiMark icon={siOracle} size={size} title="Oracle Cloud" />;
    case 'alibaba':
      return <SiMark icon={siAlibabacloud} size={size} title="Alibaba Cloud" />;
    case 'none':
      return null;
  }
}

export function OsIcon({ os, distro, size = 16 }: { os: HostOs; distro?: string; size?: number }) {
  if (distro === 'ubuntu') return <BrandLogo name="ubuntu" size={size} title="Ubuntu" />;
  if (os === 'windows') return <BrandLogo name="windows" size={size} title="Windows" />;
  if (os === 'darwin') return <BrandLogo name="darwin" size={size} title="Darwin" />;
  return <BrandLogo name="linux" size={size} title="Linux" />;
}

export function CollectorIcon({ kind, size = 16 }: { kind: CollectorKind; size?: number }) {
  if (kind === 'otel') return <BrandLogo name="otel" size={size} title="OpenTelemetry" />;
  // A neutral mark: the Datadog Agent is identified by its label, not Datadog's logo.
  if (kind === 'datadog') return <Server size={size} aria-label={COLLECTOR_LABEL.datadog} />;
  return <CollectorMark size={size} />;
}

export function SoftwareIcon({ kind, size = 16 }: { kind: Software; size?: number }) {
  return kind === 'kubernetes' ? <BrandLogo name="kubernetes" size={size} title="Kubernetes" /> : <BrandLogo name="docker" size={size} title="Docker" />;
}

export function GpuIcon({ size = 16 }: { size?: number }) {
  return <Cpu size={size} color="var(--chart-2)" aria-label="GPU" />;
}
