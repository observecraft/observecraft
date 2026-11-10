// Integrations → Microsoft Azure → an app registration → Entra ID Logs (the catalog's Microsoft Entra ID tile opens it):
// the tenant's sign-in logs, audit logs and risk detections, read from Microsoft Graph with this app registration
// (routes/entra_logs.rs). What Graph needs is said up front and checked by the test.
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Check, Info, TriangleAlert, X } from 'lucide-react';
import type { AzureAccount, EntraLogs } from '../../../api/azure/types';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { useIntegrationDashboards } from '../queries';
import { useEntraLogs, useEntraLogsMutations } from './queries';

const STREAMS: { scope: 'signins' | 'audits' | 'risk'; key: 'signIns' | 'audits' | 'risk'; label: string; hint: string }[] = [
  {
    scope: 'signins',
    key: 'signIns',
    label: 'Sign-in logs',
    hint: 'Interactive user sign-ins: who, to which application, from where, with what result. Needs Microsoft Entra ID P1 or P2 on the tenant.',
  },
  { scope: 'audits', key: 'audits', label: 'Audit logs', hint: 'Changes to users, groups, applications, roles and policies. Any license.' },
  {
    scope: 'risk',
    key: 'risk',
    label: 'Risk detections',
    hint: "Identity Protection's detections (anonymous or malicious addresses, leaked credentials, unfamiliar sign-ins). Needs Microsoft Entra ID P2 and IdentityRiskEvent.Read.All.",
  },
];

/** The Microsoft Entra ID dashboard, listed once Entra ID logs are on. */
function useDashboardLink(): string {
  const id = useIntegrationDashboards(['azure-active-directory']).data?.[0]?.id;
  return id ? `/dashboards/${id}` : '/dashboards?preset=integrations';
}

export function EntraLogsTab({ account }: { account: AzureAccount }) {
  const q = useEntraLogs(account.id);
  if (q.isPending) return <Loading label="Loading Entra ID logs…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <Form key={JSON.stringify([q.data.enabled, q.data.signIns, q.data.audits, q.data.risk])} account={account} logs={q.data} />;
}

function Form({ account, logs }: { account: AzureAccount; logs: EntraLogs }) {
  const { save, test } = useEntraLogsMutations(account.id);
  const dashboard = useDashboardLink();
  const [draft, setDraft] = useState({ enabled: logs.enabled, signIns: logs.signIns, audits: logs.audits, risk: logs.risk });
  const dirty = draft.enabled !== logs.enabled || (draft.enabled && STREAMS.some((s) => draft[s.key] !== logs[s.key]));
  const noStream = draft.enabled && !STREAMS.some((s) => draft[s.key]);
  return (
    <div className="aws-form">
      <div className="aws-note">
        <Info size={16} />
        <span>
          ObserveCraft reads the tenant&apos;s Entra ID logs from Microsoft Graph with this app registration every three minutes (no diagnostic setting, Event Hub or forwarder), as logs with source{' '}
          <code>azure.activedirectory</code>, the way Azure exports them. Grant the app registration Microsoft Graph&apos;s <strong>AuditLog.Read.All</strong> and <strong>Directory.Read.All</strong>{' '}
          application permissions (and <strong>IdentityRiskEvent.Read.All</strong> for risk detections), with admin consent.
        </span>
      </div>
      {!account.has_secret && (
        <div className="aws-issue">
          <strong>This app registration has no client secret.</strong>
          <p>Microsoft Graph is read with a client secret: add one under Details.</p>
        </div>
      )}
      <label className="aws-toggle">
        <button type="button" role="switch" aria-checked={draft.enabled} className={`switch${draft.enabled ? ' on' : ''}`} onClick={() => setDraft((d) => ({ ...d, enabled: !d.enabled }))} />
        <span>
          Collect Entra ID logs
          <small className="muted">The integration&apos;s log pipeline is installed, and the Microsoft Entra ID dashboard and Cloud SIEM rules read them.</small>
        </span>
      </label>
      {draft.enabled &&
        STREAMS.map((s) => (
          <label key={s.key} className="aws-toggle">
            <button
              type="button"
              role="switch"
              aria-label={s.label}
              aria-checked={draft[s.key]}
              className={`switch${draft[s.key] ? ' on' : ''}`}
              onClick={() => setDraft((d) => ({ ...d, [s.key]: !d[s.key] }))}
            />
            <span>
              {s.label}
              <small className="muted">{s.hint}</small>
            </span>
          </label>
        ))}
      {noStream && <p className="form-error">Choose at least one log.</p>}
      {save.error && <p className="form-error">{(save.error as Error).message}</p>}
      <div className="aws-save">
        <button className="btn" disabled={!account.has_secret || test.isPending} onClick={() => test.mutate()}>
          {test.isPending ? 'Testing…' : 'Test access'}
        </button>
        <button className="btn btn-primary" disabled={!dirty || noStream || save.isPending} onClick={() => save.mutate(draft)}>
          {save.isPending ? 'Saving…' : 'Save'}
        </button>
      </div>
      {test.error && <p className="form-error">{(test.error as Error).message}</p>}
      {test.data && (
        <table className="im-table">
          <thead>
            <tr>
              <th>LOG</th>
              <th>ACCESS</th>
              <th>NEEDS</th>
            </tr>
          </thead>
          <tbody>
            {test.data.streams.map((t) => (
              <tr key={t.stream}>
                <td>{STREAMS.find((s) => s.scope === t.stream)?.label ?? t.stream}</td>
                <td>
                  {t.ok ? <Check size={14} className="aws-ok-icon" /> : <X size={14} className="aws-warn-icon" />} {t.message}
                </td>
                <td className="muted">
                  <code>{t.permission}</code>
                  {t.license && <> · {t.license}</>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {logs.enabled && (
        <>
          <h4 className="aws-subhead">Collection</h4>
          <table className="im-table">
            <thead>
              <tr>
                <th>LOG</th>
                <th>LAST READ</th>
                <th>READ UP TO</th>
                <th>EVENTS LAST READ</th>
                <th>STATUS</th>
              </tr>
            </thead>
            <tbody>
              {STREAMS.filter((s) => logs[s.key]).map((s) => {
                const job = logs.streams.find((j) => j.scope === s.scope);
                return (
                  <tr key={s.scope}>
                    <td>{s.label}</td>
                    <td>{job?.last_success_at ? timeAgo(job.last_success_at) : '—'}</td>
                    <td>{job?.collected_until ? new Date(job.collected_until).toLocaleString() : '—'}</td>
                    <td>{job?.metrics ?? 0}</td>
                    <td>
                      {job?.error ? (
                        <span title={job.error.message}>
                          <TriangleAlert size={14} className="aws-warn-icon" /> {job.error.message}
                        </span>
                      ) : job?.last_success_at ? (
                        <Check size={14} className="aws-ok-icon" />
                      ) : (
                        'Pending'
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          <div className="aws-save">
            <Link className="btn" to={`/logs?query=${encodeURIComponent('source:azure.activedirectory')}`}>
              View logs
            </Link>
            <Link className="btn btn-primary" to={dashboard}>
              Open the dashboard
            </Link>
          </div>
        </>
      )}
    </div>
  );
}
