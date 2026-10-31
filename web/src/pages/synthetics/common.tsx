import { useState, type ReactNode } from 'react';
import { Cable, Check, ChevronDown, ChevronRight, Globe, HeartPulse, ShieldCheck, Timer, X } from 'lucide-react';
import type { MonitorStatus } from '../../api/monitors/client';
import type { SyntheticRunDetail, SyntheticTestType, SyntheticTimings, SyntheticUptimeSegment } from '../../api/synthetics/client';
import { TYPE_LABEL, assertionText, failureSummary } from '../../api/synthetics/logic';
import { StatusBadge, fmtDateTime } from '../monitors/common';

export const SEGMENT_COLOR: Record<MonitorStatus, string> = {
  ok: 'var(--mon-ok)',
  alert: 'var(--mon-alert)',
  warn: 'var(--mon-warn)',
  nodata: 'var(--mon-nodata)',
};

export function TypeBadge({ type }: { type: SyntheticTestType }) {
  return <span className={`syn-type syn-type-${type}`}>{TYPE_LABEL[type]}</span>;
}

const TYPE_ICON = { http: HeartPulse, ssl: ShieldCheck, dns: Globe, tcp: Cable };

/** The test type's icon (API tests share a pulse, as subtypes get their own). */
export function TypeIcon({ type, size = 15 }: { type: SyntheticTestType; size?: number }) {
  const Icon = TYPE_ICON[type];
  return <Icon size={size} className="syn-type-icon" />;
}

export function TestStatus({ status, paused, small }: { status: MonitorStatus; paused: boolean; small?: boolean }) {
  if (paused) return <span className={`mon-status syn-paused${small ? ' small' : ''}`}>PAUSED</span>;
  return <StatusBadge status={status} small={small} />;
}

export function PassFail({ passed }: { passed: boolean }) {
  return <span className={`syn-result ${passed ? 'pass' : 'fail'}`}>{passed ? <Check size={13} /> : <X size={13} />}</span>;
}

export function fmtMs(ms: number | null | undefined): string {
  if (ms == null) return '—';
  if (ms >= 10_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.round(ms)} ms`;
}

export function fmtBytes(n: number | null | undefined): string {
  if (n == null) return '—';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / 1024 / 1024).toFixed(1)} MiB`;
}

/** The test's state over [from, to]: one colored bar. */
export function UptimeBar({ segments, from, to }: { segments: SyntheticUptimeSegment[]; from: number; to: number }) {
  const span = Math.max(1, to - from);
  return (
    <div className="syn-uptime-bar" role="img" aria-label="Alert state over the time range">
      {segments.map((s, i) => (
        <i
          key={i}
          title={`${s.status === 'ok' ? 'OK' : s.status === 'alert' ? 'Alert' : 'No Data'}: ${fmtDateTime(s.from)} – ${fmtDateTime(s.to)}`}
          style={{ left: `${((Math.max(s.from, from) - from) / span) * 100}%`, width: `${((Math.min(s.to, to) - Math.max(s.from, from)) / span) * 100}%`, background: SEGMENT_COLOR[s.status] }}
        />
      ))}
    </div>
  );
}

/** A collapsible section of the run view (Assertions, Request Details, Response Details). */
function Fold({ title, count, open: initial = true, children }: { title: string; count?: number; open?: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(initial);
  return (
    <section className="syn-fold">
      <button className="syn-fold-head" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
        <span>{title}</span>
        {count != null && <span className="syn-count">{count}</span>}
      </button>
      {open && <div className="syn-fold-body">{children}</div>}
    </section>
  );
}

const PHASES: [keyof SyntheticTimings, string, string][] = [
  ['redirect', 'Redirect', 'var(--chart-8)'],
  ['dns', 'DNS', 'var(--chart-6)'],
  ['tcp', 'Connection', 'var(--chart-1)'],
  ['ssl', 'SSL', 'var(--chart-3)'],
  ['firstByte', 'Time to first byte', 'var(--chart-6)'],
  ['download', 'Download', 'var(--chart-1)'],
];

/** Where an HTTP run's time went: each phase's share and its place on the run's timeline. */
export function TimingWaterfall({ timings }: { timings: SyntheticTimings }) {
  const total = Math.max(timings.total, 0.001);
  const rows = PHASES.filter(([k]) => timings[k] > 0 || (k !== 'redirect' && k !== 'ssl'));
  const step = niceStep(total / 6);
  const ticks: number[] = [];
  for (let v = 0; v <= total * 1.02; v += step) ticks.push(v);
  // Room after the longest bar for its label.
  const scale = Math.max(total * 1.12, ticks[ticks.length - 1]);
  let at = 0;
  const bars = rows.map(([k, label, color]) => {
    const start = at;
    at += timings[k];
    return { k, label, color, start, value: timings[k] };
  });
  const pct = (v: number) => `${Math.min(100, (v / scale) * 100)}%`;
  const share = (v: number) => `${Math.min(100, (v / total) * 100)}%`;
  return (
    <div className="syn-wf" role="table" aria-label="Response time breakdown">
      <div className="syn-wf-row syn-wf-axis" aria-hidden>
        <span />
        <span />
        <span />
        <span className="syn-wf-track">
          {ticks.map((t) => (
            <i key={t} style={{ left: pct(t) }}>
              {fmtShortMs(t)}
            </i>
          ))}
        </span>
      </div>
      {[{ k: 'total', label: 'Total response time', color: 'color-mix(in srgb, var(--status-ok) 30%, transparent)', start: 0, value: timings.total }, ...bars].map((b) => (
        <div key={b.k} className={`syn-wf-row${b.k === 'total' ? ' total' : ''}`} role="row">
          <span role="cell">{b.label}</span>
          <span role="cell" className="syn-wf-pct">
            {((b.value / total) * 100).toFixed(1)}%
          </span>
          <span role="cell" className="syn-wf-share">
            <i style={{ width: share(b.value) }} />
          </span>
          <span role="cell" className="syn-wf-track">
            {ticks.map((t) => (
              <u key={t} style={{ left: pct(t) }} />
            ))}
            <b style={{ left: pct(b.start), width: `max(2px, ${pct(b.value)})`, background: b.color }} />
            <em style={{ left: `calc(${pct(b.start + b.value)} + 6px)` }}>{fmtShortMs(b.value)}</em>
          </span>
        </div>
      ))}
    </div>
  );
}

function niceStep(raw: number): number {
  const p = 10 ** Math.floor(Math.log10(Math.max(raw, 0.001)));
  return [1, 2, 5, 10].map((m) => m * p).find((v) => v >= raw) ?? 10 * p;
}

/** 400 µs, 7 ms, 1.2 s. */
export function fmtShortMs(ms: number): string {
  if (ms === 0) return '0 µs';
  if (ms < 1) return `${Math.round(ms * 1000)} µs`;
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}

function StatusCode({ code }: { code: number }) {
  return (
    <span className={`syn-code ${code < 300 ? 'ok' : code < 400 ? 'redirect' : 'error'}`}>
      {code} {HTTP_REASON[code] ?? ''}
    </span>
  );
}

const HTTP_REASON: Record<number, string> = {
  200: 'OK',
  201: 'Created',
  202: 'Accepted',
  204: 'No Content',
  301: 'Moved Permanently',
  302: 'Found',
  304: 'Not Modified',
  307: 'Temporary Redirect',
  308: 'Permanent Redirect',
  400: 'Bad Request',
  401: 'Unauthorized',
  403: 'Forbidden',
  404: 'Not Found',
  429: 'Too Many Requests',
  500: 'Internal Server Error',
  502: 'Bad Gateway',
  503: 'Service Unavailable',
  504: 'Gateway Timeout',
};

/** What one run sent and got back: where it went, where the time went, assertions, then the response, certificate or records. */
export function RunView({ run, type }: { run: SyntheticRunDetail; type: SyntheticTestType }) {
  const why = failureSummary(run);
  return (
    <div className="syn-run">
      {why && <p className="syn-run-error">{why}</p>}
      <div className="syn-run-facts">
        {type === 'http' && run.url && (
          <div>
            <label>URL</label>
            <span className="syn-run-url">
              <span className="syn-method-chip">{run.method}</span>
              <span className="mono syn-wrap">{run.url}</span>
              {run.statusCode != null && <StatusCode code={run.statusCode} />}
            </span>
          </div>
        )}
        {run.resolvedIp && (
          <div>
            <label>Resolved IP Address</label>
            <span className="syn-chip mono">
              <Globe size={13} /> {run.resolvedIp}
            </span>
          </div>
        )}
        {type !== 'dns' && (
          <div>
            <label>DNS Server</label>
            <b>{run.dnsServer ?? 'system resolver'}</b>
          </div>
        )}
        {run.httpVersion && (
          <div>
            <label>HTTP Version</label>
            <b>{run.httpVersion}</b>
          </div>
        )}
        {type !== 'http' && (
          <div>
            <label>Response Time</label>
            <b>{fmtMs(run.durationMs)}</b>
          </div>
        )}
      </div>
      {run.timings && (
        <section className="syn-run-section">
          <TimingWaterfall timings={run.timings} />
        </section>
      )}
      {run.assertions.length > 0 && (
        <Fold title="Assertions" count={run.assertions.length}>
          <div className="syn-asserts">
            <div className="syn-asserts-head">
              <span />
              <span>ACTUAL VALUE</span>
            </div>
            {run.assertions.map((a, i) => (
              <div key={i} className={`syn-assert ${a.passed ? 'pass' : 'fail'}`}>
                <span className="syn-assert-text">{assertionText(a.assertion)}</span>
                <span className="syn-assert-dots" />
                <span className="mono">{a.error ?? (a.actual != null ? `${a.actual}${a.assertion.type === 'responseTime' ? ' ms' : ''}` : 'nothing found')}</span>
              </div>
            ))}
          </div>
        </Fold>
      )}
      {type === 'http' && run.url && (
        <Fold title="Request Details" open={false}>
          <table className="syn-kv-table">
            <tbody>
              <tr>
                <td className="syn-col-key">Method</td>
                <td className="mono">{run.method}</td>
              </tr>
              <tr>
                <td className="syn-col-key">URL</td>
                <td className="mono syn-wrap">{run.url}</td>
              </tr>
              {run.redirects.length > 0 && (
                <tr>
                  <td className="syn-col-key">Redirects</td>
                  <td className="mono syn-wrap">{run.redirects.join(' → ')}</td>
                </tr>
              )}
            </tbody>
          </table>
        </Fold>
      )}
      {type === 'http' && run.statusCode != null && (
        <Fold title="Response Details">
          <div className="syn-resp-grid">
            <div>
              <h5>Status Code</h5>
              <StatusCode code={run.statusCode} />
            </div>
            <div>
              <h5>Response Time</h5>
              <span className="syn-chip">
                <Timer size={13} /> {fmtMs(run.durationMs)}
              </span>
            </div>
          </div>
          <h5>Headers</h5>
          {run.responseHeaders.length ? (
            <table className="syn-kv-table">
              <tbody>
                {run.responseHeaders.map((h, i) => (
                  <tr key={i}>
                    <td className="mono syn-col-key">{h.name}</td>
                    <td className="mono syn-wrap">{h.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p className="syn-placeholder">No headers were kept with this run.</p>
          )}
          <h5>
            Body{' '}
            {run.bodySize != null && (
              <span className="muted">
                {fmtBytes(run.bodySize)}
                {run.bodyTruncated ? ' (truncated at 1 MiB)' : ''}
              </span>
            )}
          </h5>
          {run.body != null ? <pre className="syn-body">{run.body || <span className="muted">(empty)</span>}</pre> : <p className="syn-placeholder">Bodies of passing scheduled runs aren't kept.</p>}
        </Fold>
      )}
      {run.certificate && (
        <Fold title="Certificate">
          <table className="syn-kv-table">
            <tbody>
              {(
                [
                  ['Subject', run.certificate.subject],
                  ['Issuer', run.certificate.issuer],
                  ['Valid from', fmtDateTime(run.certificate.notBefore)],
                  ['Valid until', `${fmtDateTime(run.certificate.notAfter)} (${run.certificate.daysLeft.toFixed(1)} days left)`],
                  ['Trusted', run.certificate.trusted ? 'Yes' : `No: ${run.certificate.trustError ?? 'unknown'}`],
                  ['Protocol', run.certificate.protocol ?? '—'],
                  ['Cipher', run.certificate.cipher ?? '—'],
                ] as [string, string][]
              ).map(([k, v]) => (
                <tr key={k}>
                  <td className="syn-col-key">{k}</td>
                  <td className="mono syn-wrap">{v}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Fold>
      )}
      {(run.records.length > 0 || run.dnsServer) && type === 'dns' && (
        <Fold title={`DNS records${run.dnsServer ? ` (from ${run.dnsServer})` : ''}`} count={run.records.length}>
          <table className="syn-kv-table">
            <tbody>
              {run.records.map((r, i) => (
                <tr key={i}>
                  <td className="mono syn-col-key">{r.type}</td>
                  <td className="mono syn-wrap">{r.value}</td>
                  <td className="muted">TTL {r.ttl}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Fold>
      )}
    </div>
  );
}
