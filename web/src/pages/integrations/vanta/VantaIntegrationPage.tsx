// Integrations → Vanta: connects the org to a private Vanta app (Build Integrations); ObserveCraft sends its compliance
// evidence to it every hour, as Vanta's own Datadog integration reads Datadog's: members for access reviews first
// (then vulnerabilities, security findings, monitors and SIEM). Vanta needs no access to ObserveCraft.
import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { AlertTriangle, Check, ChevronRight, CircleCheck, RefreshCw } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { deleteVanta, saveVanta, syncVanta, testVanta, vantaStatus, type VantaInput } from '../../../api/vanta/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { useAccess } from '../../settings/queries';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { IntegrationLogo } from '../IntegrationLogo';
import { Step } from '../Step';
import '../../settings/settings.css';
import '../integrations.css';
import '../aws/aws.css';
import '../../delivery/delivery.css';

const DESCRIPTION =
  'Vanta automates compliance (SOC 2, ISO 27001, HIPAA, …) from evidence your tools provide. ObserveCraft sends its own to a private Vanta app every hour, as Vanta’s Datadog integration reads Datadog’s: who has access, vulnerabilities, security findings, monitors, detection rules and log indexes.';

const TABS = [
  { id: 'configure', label: 'Configuration' },
  { id: 'data', label: 'Data Sent' },
] as const;
type Tab = (typeof TABS)[number]['id'];

/** The resource types ObserveCraft sends (crate::vanta::sync::TYPES): their names here and what they carry. */
const TYPES: Record<string, { name: string; what: string }> = {
  user_account: { name: 'Members', what: 'Who has access to ObserveCraft, for access reviews and offboarding reminders.' },
  vulnerable_component: { name: 'Vulnerable components', what: 'Hosts, container images, services and repositories with open vulnerabilities (needed by Vulnerabilities).' },
  package_vulnerability: {
    name: 'Vulnerabilities',
    what: 'Open vulnerabilities of host and image packages (Cloud Security) and libraries (Code Security), for the Vulnerability Inventory and its SLAs.',
  },
  security_task: { name: 'Security findings', what: 'Cloud Security’s misconfigurations, open or closed, with their priority, for the security alert SLA.' },
  monitor: { name: 'Monitors', what: 'Every monitor: what it watches, whom it notifies and whether it is muted, as infrastructure monitoring evidence.' },
  detection_rule: { name: 'Detection rules', what: 'Cloud SIEM’s log detection rules: enabled, severity, notifications.' },
  log_index: { name: 'Log indexes', what: 'Log retention, flex retention, daily limits and exclusion filters.' },
};

/** What a `user_account` carries, from the member. */
const USER_FIELDS: [string, string][] = [
  ['displayName, fullName', 'The member’s name'],
  ['accountName, email', 'Their email'],
  ['permissionLevel', 'ADMIN (Admin role), EDITOR (Standard), BASE (Read Only or custom roles)'],
  ['status', 'ACTIVE, or DEACTIVATED with when; pending invitations aren’t sent'],
  ['authMethod', 'SSO when they can only sign in through SAML or OIDC, PASSWORD otherwise'],
  ['mfaEnabled, mfaMethods', 'Whether they set up an authenticator app (OTP)'],
  ['roleDescription', 'Their roles'],
  ['createdTimestamp, lastLoginTimestamp', 'When they joined and last signed in'],
];

const statusKey = ['integrations', 'vanta'];

interface Form {
  region: string;
  clientId: string;
  clientSecret: string;
  resourceIds: Record<string, string>;
}

export function VantaIntegrationPage() {
  const [params, setParams] = useSearchParams();
  const tab: Tab = TABS.find((t) => t.id === params.get('tab'))?.id ?? 'configure';
  const status = useQuery({ queryKey: statusKey, queryFn: vantaStatus, refetchInterval: (q) => (q.state.data?.connection ? 15_000 : false) });
  const installed = !!status.data?.connection;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>Vanta</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo="vanta" name="Vanta" size={72} />
        <div>
          <h1>Vanta</h1>
          <p>{DESCRIPTION}</p>
        </div>
        {installed && (
          <span className="btn aws-installed">
            <Check size={15} /> Installed
          </span>
        )}
      </header>
      <nav className="im-tabs aws-tabs">
        {TABS.map((t) => (
          <button key={t.id} className={tab === t.id ? 'active' : ''} onClick={() => setParams(t.id === 'configure' ? {} : { tab: t.id })}>
            {t.label}
          </button>
        ))}
      </nav>
      {tab === 'data' ? <DataSent /> : status.isPending ? <Loading /> : status.isError ? <ErrorState error={status.error} onRetry={() => status.refetch()} /> : <Configure />}
    </div>
  );
}

function Configure() {
  const qc = useQueryClient();
  const refresh = () => {
    qc.invalidateQueries({ queryKey: statusKey });
    qc.invalidateQueries({ queryKey: ['integrations'] });
  };
  const status = useQuery({ queryKey: statusKey, queryFn: vantaStatus });
  const { isAdmin } = useAccess();
  const test = useMutation({ mutationFn: (input: VantaInput) => testVanta(input) });
  const save = useMutation({ mutationFn: (input: VantaInput) => saveVanta(input), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteVanta, onSuccess: refresh });
  const sync = useMutation({ mutationFn: syncVanta, onSuccess: refresh });
  const [form, setForm] = useState<Form>({ region: 'us', clientId: '', clientSecret: '', resourceIds: {} });
  const [loadedAt, setLoadedAt] = useState<number | null>(null);
  const s = status.data;
  const saved = s?.connection ?? null;

  // Fill the form once from the saved connection (and again after it changes).
  useEffect(() => {
    const at = s?.connection?.updatedAt ?? 0;
    if (s && at !== loadedAt) {
      setForm({ region: s.connection?.region ?? 'us', clientId: s.connection?.clientId ?? '', clientSecret: '', resourceIds: { ...(s.connection?.resourceIds ?? {}) } });
      setLoadedAt(at);
    }
  }, [s, loadedAt]);

  if (!s) return null;
  const set = (patch: Partial<Form>) => {
    setForm((f) => ({ ...f, ...patch }));
    test.reset();
  };
  const clientId = form.clientId.trim();
  const input: VantaInput = {
    region: form.region,
    clientId,
    clientSecret: form.clientSecret.trim() || null,
    resourceIds: Object.fromEntries(Object.entries(form.resourceIds).map(([t, id]) => [t, id.trim()])),
  };
  const secretOk = !!input.clientSecret || saved?.clientId === clientId;
  const anyType = Object.values(input.resourceIds ?? {}).some((id) => !!id);
  const ready = !!clientId && secretOk;

  return (
    <div className="ci-setup">
      <Step n={1} title="Create a private app in Vanta" open onToggle={() => {}} done={!!saved}>
        <ol className="muted">
          <li>
            In Vanta, open <b>Settings → Developer Console</b> and create an app: type <b>Build Integrations</b>, distribution <b>Private</b>.
          </li>
          <li>
            Copy its <b>client id</b> and generate a <b>client secret</b>.
          </li>
        </ol>
      </Step>

      <Step n={2} title="Add a resource per type to send" open onToggle={() => {}} done={anyType}>
        <p className="muted">
          In the app, add a resource for each type to send, with the base type shown, and copy its resourceId here; a type without one isn&apos;t sent. The last three are custom resources: create each
          with the schema shown.
        </p>
        {s.types.map((t) => (
          <div key={t.resourceType}>
            <label className="rt-field">
              <span>
                {TYPES[t.resourceType]?.name ?? t.resourceType} <span className="muted">(base type {t.baseType})</span>
              </span>
              <input
                value={form.resourceIds[t.resourceType] ?? ''}
                onChange={(e) => set({ resourceIds: { ...form.resourceIds, [t.resourceType]: e.target.value } })}
                placeholder="resourceId"
                disabled={!isAdmin}
              />
              <span className="muted">{TYPES[t.resourceType]?.what}</span>
            </label>
            {t.schema && (
              <details>
                <summary className="muted">Schema of the custom resource</summary>
                <CodeBlock code={t.schema} language="json" />
              </details>
            )}
          </div>
        ))}
      </Step>

      <Step n={3} title="Connect the app" open onToggle={() => {}} done={ready}>
        <label className="rt-field">
          <span>Vanta region</span>
          <select value={form.region} onChange={(e) => set({ region: e.target.value })} disabled={!isAdmin}>
            <option value="us">Vanta (api.vanta.com)</option>
            <option value="gov">Vanta Gov (api.vanta-gov.com)</option>
          </select>
        </label>
        <label className="rt-field">
          <span>Client id</span>
          <input value={form.clientId} onChange={(e) => set({ clientId: e.target.value })} placeholder="The app’s client id" disabled={!isAdmin} />
        </label>
        <label className="rt-field">
          <span>Client secret</span>
          <input
            type="password"
            value={form.clientSecret}
            onChange={(e) => set({ clientSecret: e.target.value })}
            autoComplete="new-password"
            placeholder={saved?.clientId === clientId && clientId ? 'Saved (leave empty to keep it)' : 'The app’s client secret'}
            disabled={!isAdmin}
          />
          <span className="muted">Stored encrypted and never shown again.</span>
        </label>
      </Step>

      <Step n={4} title="Test and save" open onToggle={() => {}} done={!!saved}>
        {!isAdmin && <p className="muted">Only organization admins can change the Vanta connection.</p>}
        {test.data && (
          <div className={test.data.ok ? 'rt-note' : 'rt-warning'}>
            {test.data.ok ? <CircleCheck size={16} /> : <AlertTriangle size={16} />}
            <div>{test.data.message}</div>
          </div>
        )}
        {[test.error, save.error, remove.error, sync.error].filter(Boolean).map((e, i) => (
          <p key={i} className="form-error">
            {(e as Error).message}
          </p>
        ))}
        {save.isSuccess && !save.isPending && <p className="muted">Saved. The first sync starts within a minute, then every hour.</p>}
        <div className="rt-create-actions">
          <button type="button" className="btn" disabled={!isAdmin || !ready || test.isPending} onClick={() => test.mutate(input)}>
            {test.isPending ? 'Testing…' : 'Test credentials'}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!isAdmin || !ready || save.isPending}
            onClick={() => save.mutate(input, { onSuccess: () => setForm((f) => ({ ...f, clientSecret: '' })) })}
          >
            {save.isPending ? 'Saving…' : saved ? 'Save changes' : 'Save and start syncing'}
          </button>
          {saved && (
            <button
              type="button"
              className="btn btn-danger"
              disabled={!isAdmin || remove.isPending}
              onClick={() => window.confirm('Disconnect Vanta? What was already sent stays in Vanta.') && remove.mutate()}
            >
              Disconnect
            </button>
          )}
        </div>
      </Step>

      <Step n={5} title="Check what Vanta received" open onToggle={() => {}} done={!!s.lastSyncAt}>
        {!saved ? (
          <p className="muted">Once the connection is saved, ObserveCraft sends every configured type, then again every hour.</p>
        ) : (
          <>
            <dl className="ci-sync">
              <dt>Vanta app</dt>
              <dd>
                {saved.clientId} <span className="muted">({saved.region === 'gov' ? 'Vanta Gov' : 'Vanta'})</span>
              </dd>
              <dt>Last sync</dt>
              <dd>{s.lastSyncAt ? timeAgo(s.lastSyncAt) : 'Waiting for the first sync…'}</dd>
            </dl>
            <table className="im-table">
              <thead>
                <tr>
                  <th>Type</th>
                  <th>Last sent</th>
                  <th>Sent</th>
                  <th>Accepted</th>
                  <th>Rejected</th>
                  <th>Note</th>
                </tr>
              </thead>
              <tbody>
                {s.types.map((t) => (
                  <tr key={t.resourceType}>
                    <td>{TYPES[t.resourceType]?.name ?? t.resourceType}</td>
                    <td>{!t.configured ? <span className="muted">Not sent (no resourceId)</span> : t.syncedAt ? timeAgo(t.syncedAt) : '—'}</td>
                    <td>{t.sent.toLocaleString()}</td>
                    <td>{t.accepted.toLocaleString()}</td>
                    <td>{t.rejected.toLocaleString()}</td>
                    <td className="muted">{t.error ?? ''}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {s.error && (
              <div className="rt-warning">
                <AlertTriangle size={16} />
                <div>
                  {s.error.message} <span className="muted">({timeAgo(s.error.at)})</span>
                </div>
              </div>
            )}
            <div className="rt-create-actions">
              <button type="button" className="btn" disabled={!isAdmin || sync.isPending} onClick={() => sync.mutate()}>
                <RefreshCw size={14} /> {sync.isPending ? 'Syncing…' : 'Sync now'}
              </button>
            </div>
          </>
        )}
      </Step>
    </div>
  );
}

function DataSent() {
  return (
    <div className="ci-setup">
      <h3>How it is sent</h3>
      <p className="muted">
        Every hour, ObserveCraft gets a token for the app and sends, for each type with a resourceId, all of that type&apos;s records: Vanta replaces what it had, so what is gone from ObserveCraft is
        gone from Vanta too. Vanta takes up to 10,000 records of a type at a time; the most important are sent first and the page says when some were left out.
      </p>
      <h3>Members</h3>
      <p className="muted">
        Each member of the organization becomes an account on Vanta&apos;s Access page (base type <code>UserAccount</code>), for access reviews and offboarding reminders.
      </p>
      <dl className="ci-sync">
        {USER_FIELDS.map(([name, text]) => (
          <div key={name} style={{ display: 'contents' }}>
            <dt>
              <code>{name}</code>
            </dt>
            <dd>{text}</dd>
          </div>
        ))}
      </dl>
      <h3>Vulnerabilities</h3>
      <p className="muted">
        Open vulnerabilities of hosts and container images (Cloud Security) and of libraries, per service and repository (Code Security), as <code>PackageVulnerability</code> resources of their{' '}
        <code>VulnerableComponent</code> (sent first): package and version, severity on Vanta&apos;s 0–10 scale (the CVSS score, else the severity&apos;s lower bound), CVE, whether a fix exists and
        the version to upgrade to. Muted and resolved ones aren&apos;t sent, so Vanta closes them.
      </p>
      <h3>Security findings</h3>
      <p className="muted">
        Cloud Security&apos;s failed checks as <code>SecurityTask</code> resources: open while the check fails, closed when it passes again or is muted (with when), priority P0 (critical) to P4, and
        the provider, account, region, resource type and check as labels. Resolved findings are sent for 90 days.
      </p>
      <h3>Monitors, detection rules and log indexes</h3>
      <p className="muted">
        Custom resources of their own in Vanta (create each with the schema shown on the Configuration tab), for Custom Tests such as &quot;every monitor notifies someone&quot;, &quot;no monitor stays
        muted&quot;, &quot;every detection rule is enabled and notifies&quot; or &quot;logs are kept at least 90 days&quot;. Cloud SIEM&apos;s notification rules aren&apos;t sent: ObserveCraft
        doesn&apos;t have them yet.
      </p>
    </div>
  );
}
