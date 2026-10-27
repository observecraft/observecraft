import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { KeyRound, Trash2 } from 'lucide-react';
import { createIngestKey, listIngestKeys, revokeIngestKey } from '../../api/ingest/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { useProfile } from './queries';
import { CopyButton, errorText } from './SecuritySettings';

const KEY = ['ingest', 'keys'];

/** Where senders reach oc-intake: the app's host on the OTLP/HTTP port unless a proxy says otherwise. */
const intakeUrl = () => `${window.location.protocol}//${window.location.hostname}:4318`;

function snippets(key: string) {
  const url = intakeUrl();
  return [
    {
      label: 'OpenTelemetry SDK (environment)',
      text: `OTEL_EXPORTER_OTLP_ENDPOINT=${url}\nOTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf\nOTEL_EXPORTER_OTLP_HEADERS=authorization=Bearer ${key}\nOTEL_SERVICE_NAME=my-service`,
    },
    {
      label: 'OpenTelemetry Collector (exporter)',
      text: `exporters:\n  otlphttp/observecraft:\n    endpoint: ${url}\n    headers:\n      authorization: "Bearer ${key}"`,
    },
    {
      label: 'JSON logs (curl)',
      text: `curl -X POST ${url}/api/v2/logs \\\n  -H "authorization: Bearer ${key}" -H "content-type: application/json" \\\n  -d '[{"message":"hello","service":"my-service","level":"info"}]'`,
    },
  ];
}

/** Organization Settings → API Keys: ingest keys that send logs, metrics and traces to this org. */
export function ApiKeysPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  const q = useQuery({ queryKey: KEY, queryFn: listIngestKeys });
  const [name, setName] = useState('');
  const [created, setCreated] = useState<string | null>(null);
  const create = useMutation({ mutationFn: createIngestKey, onSuccess: () => qc.invalidateQueries({ queryKey: KEY }) });
  const revoke = useMutation({ mutationFn: revokeIngestKey, onSuccess: () => qc.invalidateQueries({ queryKey: KEY }) });

  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">API Keys</h1>
        {!isAdmin && (
          <span className="read-only" title="Only admins can create or revoke API keys">
            Read Only
          </span>
        )}
      </div>
      <p className="muted">
        API keys send telemetry to <b>{me.data?.org.name ?? 'this organization'}</b>: the Datadog Agent and Datadog tracing libraries (as their API key), OpenTelemetry (OTLP) and JSON logs. Everything
        sent with a key belongs to this organization. To call the ObserveCraft API, use an application key (Personal Settings) instead.
      </p>
      {isAdmin && (
        <form className="security-form" onSubmit={(e) => (e.preventDefault(), create.mutate(name.trim(), { onSuccess: (k) => (setCreated(k.key), setName('')) }))}>
          <input className="settings-input" placeholder="Key name (e.g. prod-collector)" value={name} onChange={(e) => setName(e.target.value)} maxLength={100} required />
          <button className="btn btn-primary" disabled={create.isPending}>
            <KeyRound size={14} /> New API Key
          </button>
        </form>
      )}
      {created && (
        <div className="new-key">
          <b>Copy this key now — it won't be shown again.</b>
          <div className="new-key-row">
            <code>{created}</code>
            <CopyButton text={created} />
            <button className="btn btn-sm" onClick={() => setCreated(null)}>
              Done
            </button>
          </div>
          {snippets(created).map((s) => (
            <div key={s.label} className="key-snippet">
              <div className="key-snippet-head">
                <span>{s.label}</span>
                <CopyButton text={s.text} />
              </div>
              <pre>{s.text}</pre>
            </div>
          ))}
        </div>
      )}
      {(create.error || revoke.error) && <p className="form-error">{errorText(create.error ?? revoke.error)}</p>}
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.length === 0 ? (
        <p className="muted">No API keys yet. Create one to start sending data.</p>
      ) : (
        <table className="settings-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>KEY</th>
              <th>CREATED BY</th>
              <th>CREATED</th>
              <th>LAST USED</th>
              {isAdmin && <th />}
            </tr>
          </thead>
          <tbody>
            {q.data.map((k) => (
              <tr key={k.id}>
                <td>{k.name}</td>
                <td className="mono">{k.prefix}…</td>
                <td>{k.createdBy ?? <span className="muted">System</span>}</td>
                <td className="muted">{timeAgo(k.createdAt)}</td>
                <td className="muted">{k.lastUsedAt ? timeAgo(k.lastUsedAt) : 'Never'}</td>
                {isAdmin && (
                  <td className="num">
                    <button className="icon-plain" title="Revoke" onClick={() => window.confirm(`Revoke "${k.name}"? Senders using it are rejected within a minute.`) && revoke.mutate(k.id)}>
                      <Trash2 size={16} />
                    </button>
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
