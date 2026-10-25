import { Database, Globe, MessageSquare, Server } from 'lucide-react';

const COLORS = ['#3d7be0', '#e2557a', '#22a387', '#e08a1e', '#8e5ad6', '#d6457c', '#1f9bb7', '#7a9a25', '#c2185b', '#5c6bc0', '#00897b', '#ef6c00'];

/** Stable color per service name. */
export function serviceColor(name: string): string {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return COLORS[h % COLORS.length];
}

export function ServiceGlyph({ service, kind, dbSystem, size = 18 }: { service: string; kind?: string; dbSystem?: string; size?: number }) {
  const Icon = dbSystem ? Database : kind === 'producer' || kind === 'consumer' ? MessageSquare : kind === 'internal' ? Server : Globe;
  return (
    <span className="sx-glyph" style={{ background: serviceColor(service), width: size, height: size }}>
      <Icon size={size - 6} />
    </span>
  );
}

export function SpanService({ service, dbSystem, kind }: { service: string; dbSystem?: string; kind?: string }) {
  return (
    <span className="sx-svc">
      <ServiceGlyph service={service} kind={kind} dbSystem={dbSystem} />
      <span>{service}</span>
      {dbSystem && <span className="sx-svc-peer">{dbSystem}</span>}
    </span>
  );
}

const STATUS_TEXT: Record<string, string> = {
  '200': 'OK',
  '201': 'CREATED',
  '202': 'ACCEPTED',
  '204': 'NO CONTENT',
  '301': 'MOVED PERMANENTLY',
  '302': 'FOUND',
  '304': 'NOT MODIFIED',
  '400': 'BAD REQUEST',
  '401': 'UNAUTHORIZED',
  '403': 'FORBIDDEN',
  '404': 'NOT FOUND',
  '409': 'CONFLICT',
  '422': 'UNPROCESSABLE ENTITY',
  '429': 'TOO MANY REQUESTS',
  '500': 'INTERNAL SERVER ERROR',
  '502': 'BAD GATEWAY',
  '503': 'SERVICE UNAVAILABLE',
  '504': 'GATEWAY TIMEOUT',
};

export function statusTone(code: string): 'ok' | 'warn' | 'err' {
  return code.startsWith('5') ? 'err' : code.startsWith('4') ? 'warn' : 'ok';
}

export function HttpStatus({ code, long = false }: { code: string; long?: boolean }) {
  return <span className={`sx-code-badge ${statusTone(code)}`}>{long ? `${code} ${STATUS_TEXT[code] ?? ''}`.trim() : code}</span>;
}
