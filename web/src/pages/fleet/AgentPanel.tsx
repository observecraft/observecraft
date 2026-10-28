import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { ExternalLink, Info, ScrollText, X } from 'lucide-react';
import type { FleetAgentDetail, FleetCheckStatus } from '../../api/fleet/types';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { ago, kindLabel } from './fleetModel';
import { IntegrationLogos, KindIcon, OsCell, StatusPill, VersionCell } from './fleetUi';
import { useFleetAgent } from './queries';

/** Side panel of one agent: what it is, its checks, its reported configuration and recent errors. */
export function AgentPanel({ id, onClose }: { id: string; onClose: () => void }) {
  const q = useFleetAgent(id);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !(e.target as HTMLElement).closest('input, textarea') && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return (
    <aside className="fl-panel" role="dialog" aria-label={`Agent ${id}`}>
      {q.isPending ? <Loading label="Loading agent…" /> : q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <Body detail={q.data} onClose={onClose} />}
    </aside>
  );
}

const CHECK_CLASS: Record<FleetCheckStatus, string> = { OK: 'ok', WARNING: 'issues', ERROR: 'error' };
const SERVICE_CLASS: Record<string, string> = { OK: 'ok', WARNING: 'issues', CRITICAL: 'error', UNKNOWN: 'offline' };

function Body({ detail, onClose }: { detail: FleetAgentDetail; onClose: () => void }) {
  const navigate = useNavigate();
  const a = detail.agent;
  const info: [string, React.ReactNode][] = [
    ['Type', kindLabel(a.kind)],
    ['Version', <VersionCell key="v" agent={a} />],
    ['OS', <OsCell key="os" agent={a} />],
    ['Install method', a.installMethod ?? <span className="muted">not reported</span>],
    ['Env', a.env ?? <span className="muted">—</span>],
    ['Cluster', a.cluster ?? <span className="muted">—</span>],
    ['Last reported', a.lastReported ? `${ago(a.lastReported)} (${new Date(a.lastReported).toLocaleString()})` : '—'],
  ];
  return (
    <div className="fl-panel-scroll">
      <header className="fl-panel-head">
        <KindIcon kind={a.kind} size={22} />
        <div className="fl-panel-title">
          <h2>{a.hostname}</h2>
          <StatusPill status={a.status} />
        </div>
        <button className="btn btn-sm" onClick={() => navigate(`/infrastructure?host=${encodeURIComponent(a.hostname)}`)}>
          View host <ExternalLink size={13} />
        </button>
        <button className="icon-btn" onClick={onClose} title="Close (Esc)">
          <X size={16} />
        </button>
      </header>

      {a.issues.length > 0 && (
        <section className="fl-section">
          <h3>Issues</h3>
          <ul className="fl-issues">
            {a.issues.map((i) => (
              <li key={i}>{i}</li>
            ))}
          </ul>
        </section>
      )}

      <section className="fl-section">
        <h3>Agent</h3>
        <dl className="fl-info">
          {info.map(([k, v]) => (
            <div key={k}>
              <dt>{k}</dt>
              <dd>{v}</dd>
            </div>
          ))}
        </dl>
      </section>

      {a.kind === 'datadog_agent' && (
        <section className="fl-section">
          <h3>Running checks</h3>
          <div className="fl-panel-integrations">
            <IntegrationLogos integrations={a.integrations} max={12} />
          </div>
          {detail.checks.length === 0 ? (
            <p className="muted">The Agent hasn’t reported its checks yet (it does every 10 minutes).</p>
          ) : (
            <table className="fl-table fl-table-compact">
              <thead>
                <tr>
                  <th>CHECK</th>
                  <th>STATUS</th>
                  <th>INSTANCES</th>
                  <th>MESSAGE</th>
                </tr>
              </thead>
              <tbody>
                {detail.checks.map((c) => (
                  <tr key={c.name}>
                    <td className="mono">{c.name}</td>
                    <td>
                      <span className={`fl-dot fl-dot-${CHECK_CLASS[c.status]}`} /> {c.loadError ? 'Failed to load' : c.status}
                    </td>
                    <td className="num">{c.instances}</td>
                    <td className="fl-message">{c.message ?? <span className="muted">—</span>}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      )}

      {detail.serviceChecks.length > 0 && (
        <section className="fl-section">
          <h3>Service checks</h3>
          <ul className="fl-service-checks">
            {detail.serviceChecks.map((s) => (
              <li key={s.name}>
                <span className={`fl-dot fl-dot-${SERVICE_CLASS[s.status] ?? 'offline'}`} />
                <span className="mono">{s.name}</span>
                <span className="muted">{s.status}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section className="fl-section">
        <h3>Configuration</h3>
        {detail.configYaml ? (
          <>
            <p className="fl-note">
              <Info size={14} /> As the agent reports it
              {detail.fullConfigReported ? '' : ' (setup and enabled features; the Agent sends its full configuration when inventories_configuration_enabled is on)'}. API keys, passwords, tokens and
              other secrets are redacted.
            </p>
            <CodeBlock code={detail.configYaml} filename="reported configuration (read-only)" />
          </>
        ) : (
          <p className="muted">This collector doesn’t report its configuration. It lives where the collector is deployed (the observecraft-agent chart values or your collector YAML).</p>
        )}
      </section>

      <section className="fl-section">
        <div className="fl-section-head">
          <h3>Recent errors in the agent’s logs</h3>
          <button className="btn btn-sm" onClick={() => navigate(`/logs?query=${encodeURIComponent(detail.logsQuery)}`)}>
            <ScrollText size={13} /> View agent logs
          </button>
        </div>
        {detail.errors.length === 0 ? (
          <p className="muted">No errors in the agent’s own logs in this time range{a.kind === 'datadog_agent' ? ' (collect them with logs_config and the datadog-agent service).' : '.'}</p>
        ) : (
          <ul className="fl-errors">
            {detail.errors.map((e, i) => (
              <li key={i}>
                <span className="muted">{new Date(e.timestamp).toLocaleTimeString()}</span>
                <span className="mono">{e.message}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
