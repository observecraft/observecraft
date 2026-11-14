import { useState } from 'react';
import { Link } from 'react-router-dom';
import { CheckCircle2, CircleDashed } from 'lucide-react';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { ServiceIcon } from '../apm/visuals';
import { useAppsecServices } from './appsec/queries';
import { usePostureScans } from './cloud/queries';
import { useWpAgentsList } from './workload/overviewQueries';
import { agentCoverage, appsecEnv, cloudAgentEnv, cloudHelmValues, IAST_LANGUAGES, workloadAgentEnv, workloadHelmValues } from './setupSnippets';
import './cloud/cloud.css';
import './security.css';

const LANGUAGES: [string, string][] = [
  ['java', 'Java'],
  ['python', 'Python'],
  ['nodejs', 'Node.js'],
  ['dotnet', '.NET'],
  ['go', 'Go'],
  ['ruby', 'Ruby'],
  ['php', 'PHP'],
];

function On({ on, label }: { on: boolean; label?: string }) {
  return (
    <span className={`sec-setup-flag${on ? ' on' : ''}`}>
      {on ? <CheckCircle2 size={14} /> : <CircleDashed size={14} />} {label ?? (on ? 'On' : 'Off')}
    </span>
  );
}

/** App & API Protection → Settings: the tracer variables per language, and the services sending security traces. */
export function AppsecSetupPage() {
  const [language, setLanguage] = useState('java');
  const [on, setOn] = useState({ threats: true, sca: true, iast: false });
  const services = useAppsecServices();
  const env = appsecEnv(language, on);
  const iast = IAST_LANGUAGES.includes(language);
  return (
    <div className="sec-page sec-setup">
      <h2 className="sec-h">Set up App &amp; API Protection</h2>
      <section className="sec-setup-box">
        <h4>1. Turn it on in the tracer</h4>
        <p className="muted">Set these variables on the service (its container, deployment or process) and restart it. The tracer then sends security traces with the APM ones.</p>
        <div className="sec-setup-row">
          <label>
            Language{' '}
            <select value={language} onChange={(e) => setLanguage(e.target.value)}>
              {LANGUAGES.map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label>
            <input type="checkbox" checked={on.threats} onChange={(e) => setOn({ ...on, threats: e.target.checked })} /> Threat detection
          </label>
          <label>
            <input type="checkbox" checked={on.sca} onChange={(e) => setOn({ ...on, sca: e.target.checked })} /> Software Composition Analysis
          </label>
          <label title={iast ? '' : 'This tracer has no runtime code analysis'}>
            <input type="checkbox" disabled={!iast} checked={on.iast && iast} onChange={(e) => setOn({ ...on, iast: e.target.checked })} /> Runtime code analysis (IAST)
          </label>
        </div>
        {env.length ? <CodeBlock code={env.join('\n')} filename="environment" /> : <p className="muted">Pick at least one capability.</p>}
        <p className="muted">In-app blocking (WAF policies, the denylist) needs Remote Configuration, which ObserveCraft doesn&rsquo;t serve yet: rules detect attacks, they don&rsquo;t block them.</p>
      </section>
      <section className="sec-setup-box">
        <h4>2. Services with App &amp; API Protection</h4>
        {services.isLoading ? (
          <Loading />
        ) : services.isError ? (
          <ErrorState error={services.error} onRetry={() => services.refetch()} />
        ) : !services.data?.length ? (
          <p className="muted">No service sends security traces yet.</p>
        ) : (
          <table className="mon-table syn-table">
            <thead>
              <tr>
                <th>Service</th>
                <th>Env</th>
                <th>Threat detection</th>
                <th>SCA</th>
                <th>IAST</th>
                <th>Security traces</th>
              </tr>
            </thead>
            <tbody>
              {services.data.map((s) => (
                <tr key={`${s.service}:${s.env}`}>
                  <td>
                    <span className="sec-setup-svc">
                      <ServiceIcon name={s.service} kind="web" language={s.language} /> {s.service}
                    </span>
                  </td>
                  <td>{s.env || '—'}</td>
                  <td>
                    <On on={s.protected || s.securityTraces > 0} />
                  </td>
                  <td>
                    <On on={s.sca} />
                  </td>
                  <td>
                    <On on={s.iast} label={s.iast ? 'On' : IAST_LANGUAGES.includes(s.language) ? 'Off' : 'n/a'} />
                  </td>
                  <td>{s.securityTraces}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}

/** Cloud Security → Settings: the Agent's SBOMs and compliance checks, and the cloud accounts' scans. */
export function CloudSecuritySetupPage() {
  const agents = useWpAgentsList();
  const scans = usePostureScans();
  const all = [...(agents.data?.hosts ?? []), ...(agents.data?.serverless ?? [])];
  const cov = agentCoverage(all);
  const accounts = scans.data?.accounts ?? [];
  return (
    <div className="sec-page sec-setup">
      <h2 className="sec-h">Set up Cloud Security</h2>
      <section className="sec-setup-box">
        <h4>1. Vulnerabilities and compliance from the Agent</h4>
        <p className="muted">The Agent sends a bill of materials (SBOM) of its host and container images, matched against advisories, and runs the host compliance benchmarks.</p>
        {agents.isLoading ? (
          <Loading />
        ) : (
          <p>
            <b>{cov.sbom}</b> of <b>{cov.total}</b> Agents send SBOMs, <b>{cov.compliance}</b> run compliance checks.
          </p>
        )}
        <div className="sec-setup-grid">
          <CodeBlock code={cloudHelmValues()} filename="values.yaml (datadog chart)" />
          <CodeBlock code={cloudAgentEnv().join('\n')} filename="Agent environment (hosts)" />
        </div>
      </section>
      <section className="sec-setup-box">
        <h4>2. Misconfigurations of cloud accounts</h4>
        <p className="muted">
          ObserveCraft&rsquo;s scanner checks AWS accounts with resource collection on. Azure subscriptions and Google Cloud projects aren&rsquo;t scanned yet: their resources are listed from the
          catalog only.
        </p>
        {scans.isLoading ? (
          <Loading />
        ) : !accounts.length ? (
          <p>
            No AWS account connected: connect one in <Link to="/integrations/aws">Integrations → AWS</Link> and turn on resource collection.
          </p>
        ) : (
          <table className="mon-table syn-table">
            <thead>
              <tr>
                <th>Account</th>
                <th>Scanning</th>
                <th>Last scan</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {accounts.map((a) => (
                <tr key={a.accountRef}>
                  <td>
                    <span className="sec-setup-svc">
                      <BrandLogo name="aws" size={16} /> <span className="mono">{a.accountId}</span>
                    </span>
                  </td>
                  <td>
                    <On on={a.enabled} />
                  </td>
                  <td>
                    {!a.enabled
                      ? '—'
                      : a.status === 'done'
                        ? `${timeAgo(a.finishedAt ?? Date.now())}: ${a.failed ?? 0} failing, ${a.passed ?? 0} passing`
                        : a.status === 'failed'
                          ? `failed: ${a.error ?? ''}`
                          : (a.status ?? 'not scanned yet')}
                  </td>
                  <td>
                    <Link to="/integrations/aws">Resource collection</Link>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {scans.data && !scans.data.scannerConfigured && accounts.some((a) => a.enabled) && (
          <p className="muted">No scanner runs for this install: deploy it with the server&rsquo;s OC_SCANNER_TOKEN (the chart&rsquo;s scanner.enabled).</p>
        )}
      </section>
    </div>
  );
}

/** Workload Protection → Setup: runtime security on the Agent, and which Agents run it. */
export function WorkloadSetupPage() {
  const agents = useWpAgentsList();
  const hosts = agents.data?.hosts ?? [];
  const cov = agentCoverage(hosts);
  return (
    <div className="sec-page sec-setup">
      <h2 className="sec-h">Set up Workload Protection</h2>
      <section className="sec-setup-box">
        <h4>1. Runtime security on the Agent</h4>
        <p className="muted">The security agent and system-probe watch file, process, network and kernel activity; detection rules raise signals from it.</p>
        <div className="sec-setup-grid">
          <CodeBlock code={workloadHelmValues()} filename="values.yaml (datadog chart)" />
          <CodeBlock code={workloadAgentEnv().join('\n')} filename="Agent environment (hosts)" />
        </div>
      </section>
      <section className="sec-setup-box">
        <h4>
          2. Agents ({cov.runtime} of {cov.total} with runtime security)
        </h4>
        {agents.isLoading ? (
          <Loading />
        ) : agents.isError ? (
          <ErrorState error={agents.error} onRetry={() => agents.refetch()} />
        ) : !hosts.length ? (
          <p className="muted">No Agent reports yet.</p>
        ) : (
          <table className="mon-table syn-table">
            <thead>
              <tr>
                <th>Host</th>
                <th>OS</th>
                <th>Agent</th>
                <th>Runtime security</th>
                <th>Remote configuration</th>
                <th>Last seen</th>
              </tr>
            </thead>
            <tbody>
              {hosts.map((a) => (
                <tr key={a.id}>
                  <td className="mono">{a.hostname}</td>
                  <td>
                    <span className="sec-setup-svc">
                      {a.osLogo && <BrandLogo name={a.osLogo} size={16} />} {a.os || '—'}
                    </span>
                  </td>
                  <td>{a.agentVersion}</td>
                  <td>
                    <On on={a.isCwsEnabled} />
                  </td>
                  <td>
                    <On on={a.isCwsRemoteConfigurationEnabled} />
                  </td>
                  <td>{a.lastSeen ? timeAgo(a.lastSeen) : '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
