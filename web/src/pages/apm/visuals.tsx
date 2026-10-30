// Shared APM marks and number formats: service/kind icons, health badge, latency and rate text,
// line sparklines and starred services.
import { useCallback, useState } from 'react';
import { Database, Globe, Layers, MemoryStick, Server, Workflow } from 'lucide-react';
import type { Health, ServiceKind } from '../../api/apm/client';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';

export const KIND_LABEL: Record<ServiceKind, string> = { web: 'Web', worker: 'Worker', db: 'Database', cache: 'Cache', queue: 'Queue', http: 'External HTTP' };

const KIND_ICON = { web: Globe, worker: Workflow, db: Database, cache: MemoryStick, queue: Layers, http: Server } as const;

// OTel `telemetry.sdk.language` / inferred `peer.service` → BrandLogo key.
const LANGUAGE_LOGO: Record<string, string> = { java: 'openjdk', go: 'go', nodejs: 'nodedotjs', python: 'python', dotnet: 'dotnet' };
const PEER_LOGO: Record<string, string> = {
  mongo: 'mongodb',
  mongodb: 'mongodb',
  redis: 'redis',
  postgres: 'postgresql',
  postgresql: 'postgresql',
  oracle: 'oracle',
  kafka: 'apachekafka',
  'sendgrid-api': 'sendgrid',
};

// `db.system` / `messaging.system` of inferred services → BrandLogo key.
const TECH_LOGO: Record<string, string> = {
  postgresql: 'postgresql',
  mysql: 'mysql',
  mariadb: 'mysql',
  redis: 'redis',
  mongodb: 'mongodb',
  elasticsearch: 'elasticsearch',
  cassandra: 'apachecassandra',
  dynamodb: 'amazondynamodb',
  oracle: 'oracle',
  kafka: 'apachekafka',
  rabbitmq: 'rabbitmq',
};

export const LANGUAGE_LABEL: Record<string, string> = { java: 'Java', go: 'Go', nodejs: 'Node.js', python: 'Python', dotnet: '.NET' };

/** The logo of a service: its language, the technology of an inferred peer, or a known peer name. */
export function serviceLogo(name: string, language?: string | null, technology?: string | null): string | null {
  const key = (language && LANGUAGE_LOGO[language]) || (technology && TECH_LOGO[technology]) || PEER_LOGO[name];
  return key && hasBrandLogo(key) ? key : null;
}

/** Square kind badge (web/db/…) and, when known, the language or technology logo. */
export function ServiceIcon({ name, kind, language, technology, size = 16 }: { name: string; kind: ServiceKind; language?: string | null; technology?: string | null; size?: number }) {
  const Icon = KIND_ICON[kind];
  const logo = serviceLogo(name, language, technology);
  return (
    <span className="svc-icon">
      <span className={`svc-kind svc-kind-${kind}`} title={KIND_LABEL[kind]}>
        <Icon size={size - 4} />
      </span>
      {logo && <BrandLogo name={logo} size={size} title={language ? (LANGUAGE_LABEL[language] ?? language) : (technology ?? name)} />}
    </span>
  );
}

export function HealthBadge({ health }: { health: Health }) {
  return <span className={`health-badge health-${health}`}>{health === 'ok' ? 'OK' : health === 'warn' ? 'WARN' : 'CRITICAL'}</span>;
}

/** 91.35 → "91.4 ms", 1830 → "1.83 s", 0.42 → "420 µs". */
export function fmtLatency(ms: number): string {
  if (ms < 1) return `${Math.round(ms * 1000)} µs`;
  if (ms < 1000) return `${ms < 10 ? ms.toFixed(2) : ms < 100 ? ms.toFixed(1) : Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}

export function fmtRate(perSec: number): string {
  if (perSec === 0) return '0 req/s';
  if (perSec < 0.01) return `${(perSec * 60).toFixed(2)} req/min`;
  return `${perSec < 10 ? perSec.toFixed(2) : perSec < 100 ? perSec.toFixed(1) : Math.round(perSec)} req/s`;
}

/** Fraction → "1.43%"; tiny non-zero rates stay visible as "<0.01%". */
export function fmtPct(frac: number): string {
  if (frac === 0) return '0%';
  const p = frac * 100;
  if (p < 0.01) return '<0.01%';
  return `${p < 10 ? p.toFixed(2) : p.toFixed(1)}%`;
}

export function errorTone(frac: number): string {
  return frac >= 0.05 ? 'tone-error' : frac >= 0.01 ? 'tone-warn' : '';
}

/** Small line sparkline; `tone` picks the stroke. Inline SVG — tables render dozens. */
export function LineSpark({ values, tone = 'primary', width = 96, height = 24 }: { values: number[]; tone?: 'primary' | 'error'; width?: number; height?: number }) {
  if (values.length < 2) return <svg width={width} height={height} aria-hidden />;
  const max = Math.max(...values);
  const min = Math.min(0, ...values);
  const span = max - min || 1;
  const step = width / (values.length - 1);
  const pts = values.map((v, i) => `${(i * step).toFixed(1)},${(height - 2 - ((v - min) / span) * (height - 4)).toFixed(1)}`).join(' ');
  return (
    <svg width={width} height={height} className="line-spark" aria-hidden>
      <polyline points={pts} fill="none" stroke={tone === 'error' ? 'var(--status-error)' : 'var(--chart-1)'} strokeWidth="1.5" strokeLinejoin="round" />
    </svg>
  );
}

const STAR_KEY = 'oc.apm.starred';

function readStars(key: string): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(key) ?? '[]');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

/** Starred services (or, with another key, resources): a per-browser convenience, kept in localStorage. */
export function useStars(key = STAR_KEY) {
  const [stars, setStars] = useState<Set<string>>(() => new Set(readStars(key)));
  const toggle = useCallback(
    (name: string) => {
      setStars((prev) => {
        const next = new Set(prev);
        if (!next.delete(name)) next.add(name);
        try {
          localStorage.setItem(key, JSON.stringify([...next]));
        } catch {
          /* storage unavailable: stars last for this session only */
        }
        return next;
      });
    },
    [key],
  );
  return { stars, toggle };
}
