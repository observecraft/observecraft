import { useState } from 'react';
import { Check, X } from 'lucide-react';
import type { MonitorStatus } from '../../api/monitors/client';
import type { SyntheticRunDetail, SyntheticTestType, SyntheticUptimeSegment } from '../../api/synthetics/client';
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

/** What one run sent and got back: summary, assertions, then the response, certificate or records. */
export function RunView({ run, type }: { run: SyntheticRunDetail; type: SyntheticTestType }) {
  const [showBody, setShowBody] = useState(true);
  const why = failureSummary(run);
  return (
    <div className="syn-run">
      <div className={`syn-run-summary ${run.passed ? 'pass' : 'fail'}`}>
        <PassFail passed={run.passed} />
        <b>{run.passed ? 'Passed' : 'Failed'}</b>
        {run.statusCode != null && <span className="syn-chip">HTTP {run.statusCode}</span>}
        <span className="syn-chip">{fmtMs(run.durationMs)}</span>
        {run.resolvedIp && <span className="syn-chip mono">{run.resolvedIp}</span>}
        {run.bodySize != null && (
          <span className="syn-chip">
            {fmtBytes(run.bodySize)}
            {run.bodyTruncated ? ' (truncated at 1 MiB)' : ''}
          </span>
        )}
      </div>
      {why && <p className="syn-run-error">{why}</p>}
      {run.assertions.length > 0 && (
        <section className="syn-run-section">
          <h4>Assertions</h4>
          <table className="syn-kv-table">
            <tbody>
              {run.assertions.map((a, i) => (
                <tr key={i}>
                  <td className="syn-col-icon">
                    <PassFail passed={a.passed} />
                  </td>
                  <td>{assertionText(a.assertion)}</td>
                  <td className="muted mono">{a.error ?? (a.actual != null ? a.actual : 'nothing found')}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
      {type === 'http' && run.url && (
        <section className="syn-run-section">
          <h4>Request</h4>
          <p className="mono syn-wrap">
            {run.method} {run.url}
          </p>
          {run.redirects.length > 0 && (
            <p className="muted">
              Followed {run.redirects.length} redirect{run.redirects.length > 1 ? 's' : ''}: {run.redirects.join(' → ')}
            </p>
          )}
        </section>
      )}
      {run.responseHeaders.length > 0 && (
        <section className="syn-run-section">
          <h4>Response headers</h4>
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
        </section>
      )}
      {run.body != null && (
        <section className="syn-run-section">
          <h4>
            <button className="link-btn" onClick={() => setShowBody((s) => !s)}>
              Response body {showBody ? '▾' : '▸'}
            </button>
          </h4>
          {showBody && <pre className="syn-body">{run.body || <span className="muted">(empty)</span>}</pre>}
        </section>
      )}
      {run.certificate && (
        <section className="syn-run-section">
          <h4>Certificate</h4>
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
        </section>
      )}
      {(run.records.length > 0 || run.dnsServer) && (
        <section className="syn-run-section">
          <h4>DNS records{run.dnsServer ? ` (from ${run.dnsServer})` : ''}</h4>
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
        </section>
      )}
    </div>
  );
}
