// Integrations → FleetDM: connects a Fleet server (self-hosted or Fleet's cloud) with an API-only user's token;
// ObserveCraft polls its activities, hosts and policies every minute and stores them as logs (`source:fleetdm`) and
// `fleetdm.*` metrics, charted by the FleetDM Overview dashboard. Fleet needs no access to ObserveCraft.
import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { AlertTriangle, Check, ChevronRight, CircleCheck, RefreshCw } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { deleteFleetdm, fleetdmStatus, saveFleetdm, syncFleetdm, testFleetdm, type FleetdmInput } from '../../../api/fleetdm/client';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { useAccess } from '../../settings/queries';
import { IntegrationLogo } from '../IntegrationLogo';
import { useIntegrationDashboards } from '../queries';
import { Step } from '../Step';
import '../../settings/settings.css';
import '../integrations.css';
import '../aws/aws.css';
import '../../delivery/delivery.css';

const DESCRIPTION =
  'Fleet (FleetDM) is an open-source device management platform built on osquery: it inventories and queries laptops, desktops and servers, checks them against policies and manages their MDM. ObserveCraft reads a Fleet server’s API every minute: its activity feed (logins, policies and queries changed, live queries, scripts run, software installed, hosts locked or wiped) as logs, and every host’s state (online, last seen, OS version, disk space, failing policies, vulnerabilities, MDM enrollment) and the policies’ passing and failing hosts as metrics, with a dashboard over both.';

/** The org's FleetDM dashboard (its id carries the org), once Fleet is connected. */
function useDashboardLink(): string {
  const dashboards = useIntegrationDashboards(['fleetdm']);
  const id = dashboards.data?.[0]?.id;
  return id ? `/dashboards/${id}` : '/dashboards?preset=integrations';
}

const TABS = [
  { id: 'configure', label: 'Configuration' },
  { id: 'data', label: 'Data Collected' },
] as const;
type Tab = (typeof TABS)[number]['id'];

/** Searches over the activities, as the Data Collected tab links them. */
const SEARCHES: { title: string; text: string; query: string }[] = [
  {
    title: 'Logins',
    text: 'Logins to Fleet, failed logins (errors, with the client IP) and users added by SSO.',
    query: 'source:fleetdm @evt.name:(user_logged_in OR user_failed_login OR user_added_by_sso)',
  },
  { title: 'Policies and queries', text: 'Policies and saved queries created, edited or deleted, and live queries run.', query: 'source:fleetdm @evt.name:(*policy* OR *saved_query* OR live_query)' },
  {
    title: 'Actions on hosts',
    text: 'Scripts run, software installed or uninstalled, hosts locked, unlocked or wiped, disk encryption keys read.',
    query: 'source:fleetdm @fleetdm.host_display_name:*',
  },
  { title: 'Users and teams', text: 'Users created or deleted, roles changed, teams created and hosts transferred.', query: 'source:fleetdm @evt.name:(*user* OR *team* OR transferred_hosts)' },
  { title: 'Failures', text: 'Failed logins and failed software installs or scripts.', query: 'source:fleetdm status:error' },
  { title: 'Warnings', text: 'Hosts locked or wiped, disk encryption keys read, and anything deleted.', query: 'source:fleetdm status:warn' },
];

const ATTRIBUTES: [string, string][] = [
  ['evt.name', 'Fleet’s activity type (created_policy, ran_script, user_failed_login, …)'],
  ['evt.outcome', 'success or failure'],
  ['usr.id, usr.name, usr.email', 'Who did it; absent for Fleet’s own activities'],
  ['network.client.ip', 'The client’s address, for logins'],
  ['fleetdm.host_id, fleetdm.host_display_name', 'The host the activity was about'],
  ['fleetdm.team, fleetdm.fleet_initiated', 'The team involved; whether Fleet itself did it (automations)'],
  ['fleetdm.category', 'authentication, host_action, software, mdm, policy, query, script, user_management, configuration or other (the log pipeline)'],
  ['fleetdm.details', 'The activity’s details, as Fleet records them'],
];

const METRICS: [string, string][] = [
  ['fleetdm.hosts.total, .online, .offline, .mia, .new, .missing_30_days, .low_disk_space', 'Fleet’s host counts (low disk space: Fleet Premium)'],
  ['fleetdm.hosts.by_platform', 'Hosts per platform (tag platform)'],
  ['fleetdm.host.count', '1 per host: summed, the hosts (by os_version, osquery_version, team, …)'],
  ['fleetdm.host.online, .offline, .missing', '1 for the host’s status, else 0'],
  ['fleetdm.host.seen_age', 'Seconds since the host last checked in'],
  ['fleetdm.host.uptime, fleetdm.host.memory', 'Uptime (seconds) and memory (bytes)'],
  ['fleetdm.host.disk.available, .disk.total, .disk.available_percent', 'Disk space, as osquery reports it'],
  ['fleetdm.host.failing_policies, .critical_vulnerabilities, .issues', 'Fleet’s issues for the host (critical vulnerabilities: Fleet Premium)'],
  ['fleetdm.host.mdm_enrolled, fleetdm.host.disk_encrypted', '1 or 0, for hosts whose MDM or disk encryption status Fleet knows'],
  ['fleetdm.policy.passing_hosts, fleetdm.policy.failing_hosts', 'Per global policy (tags policy, policy_id, critical, platform), as Fleet last counted them'],
  ['fleetdm.vulnerabilities.total', 'Every vulnerability (CVE) Fleet found on the hosts'],
  ['fleetdm.vulnerability.hosts', 'Hosts with each of the 50 most widespread vulnerabilities (tags cve, severity from its CVSS score, known_exploited from CISA’s catalog; scores: Fleet Premium)'],
];

const statusKey = ['integrations', 'fleetdm'];

interface Form {
  baseUrl: string;
  token: string;
}

export function FleetdmIntegrationPage() {
  const [params, setParams] = useSearchParams();
  const tab: Tab = TABS.find((t) => t.id === params.get('tab'))?.id ?? 'configure';
  const status = useQuery({ queryKey: statusKey, queryFn: fleetdmStatus, refetchInterval: (q) => (q.state.data?.connection ? 15_000 : false) });
  const installed = !!status.data?.connection;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>FleetDM</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo="fleetdm" name="FleetDM" size={72} />
        <div>
          <h1>FleetDM</h1>
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
      {tab === 'data' ? <DataCollected /> : status.isPending ? <Loading /> : status.isError ? <ErrorState error={status.error} onRetry={() => status.refetch()} /> : <Configure />}
    </div>
  );
}

function Configure() {
  const qc = useQueryClient();
  const refresh = () => {
    qc.invalidateQueries({ queryKey: statusKey });
    qc.invalidateQueries({ queryKey: ['dashboards'] });
  };
  const status = useQuery({ queryKey: statusKey, queryFn: fleetdmStatus });
  const { isAdmin } = useAccess();
  const dashboard = useDashboardLink();
  const test = useMutation({ mutationFn: (input: FleetdmInput) => testFleetdm(input) });
  const save = useMutation({ mutationFn: (input: FleetdmInput) => saveFleetdm(input), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteFleetdm, onSuccess: refresh });
  const sync = useMutation({ mutationFn: syncFleetdm, onSuccess: refresh });
  const [form, setForm] = useState<Form>({ baseUrl: '', token: '' });
  const [loadedAt, setLoadedAt] = useState<number | null>(null);
  const s = status.data;
  const saved = s?.connection ?? null;

  // Fill the form once from the saved connection (and again after it changes).
  useEffect(() => {
    const at = s?.connection?.updatedAt ?? 0;
    if (s && at !== loadedAt) {
      setForm({ baseUrl: s.connection?.baseUrl ?? '', token: '' });
      setLoadedAt(at);
    }
  }, [s, loadedAt]);

  if (!s) return null;
  const set = (patch: Partial<Form>) => {
    setForm((f) => ({ ...f, ...patch }));
    test.reset();
  };
  const baseUrl = form.baseUrl.trim().replace(/\/+$/, '');
  const input: FleetdmInput = { baseUrl, token: form.token.trim() || null };
  const urlOk = /^https?:\/\/[^\s/]+/.test(baseUrl);
  const tokenOk = !!input.token || saved?.baseUrl === baseUrl;
  const ready = urlOk && tokenOk;

  return (
    <div className="ci-setup">
      <Step n={1} title="Create a read-only API user in Fleet" open onToggle={() => {}} done={!!saved}>
        <ol className="muted">
          <li>
            Create an <b>API-only user</b> with the global <b>Observer</b> role (read-only: hosts, policies and the activity feed), e.g. with fleetctl:
            <CodeBlock code="fleetctl user create --name ObserveCraft --email observecraft@example.com --api-only --global-role observer" filename="Terminal" />
          </li>
          <li>
            Copy the API token fleetctl prints (or log in as the user and copy it from <b>My account → Get API token</b>). API-only users&apos; tokens don&apos;t expire with sessions.
          </li>
        </ol>
      </Step>

      <Step n={2} title="Connect the Fleet server" open onToggle={() => {}} done={urlOk && tokenOk}>
        <label className="rt-field">
          <span>Fleet URL</span>
          <input value={form.baseUrl} onChange={(e) => set({ baseUrl: e.target.value })} placeholder="https://fleet.example.com" disabled={!isAdmin} />
          <span className="muted">The address you open Fleet at. ObserveCraft calls its API (/api/v1/fleet) from its servers over HTTPS.</span>
        </label>
        <label className="rt-field">
          <span>API token</span>
          <input
            type="password"
            value={form.token}
            onChange={(e) => set({ token: e.target.value })}
            autoComplete="new-password"
            placeholder={saved?.baseUrl === baseUrl ? 'Saved (leave empty to keep it)' : 'The API-only user’s token'}
            disabled={!isAdmin}
          />
          <span className="muted">Stored encrypted and never shown again.</span>
        </label>
      </Step>

      <Step n={3} title="Test and save" open onToggle={() => {}} done={!!saved}>
        {!isAdmin && <p className="muted">Only organization admins can change the Fleet connection.</p>}
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
        {save.isSuccess && !save.isPending && <p className="muted">Saved. The first sync starts within a minute; the FleetDM log pipeline and dashboard are installed.</p>}
        <div className="rt-create-actions">
          <button type="button" className="btn" disabled={!isAdmin || !ready || test.isPending} onClick={() => test.mutate(input)}>
            {test.isPending ? 'Testing…' : 'Test connection'}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!isAdmin || !ready || save.isPending}
            onClick={() => save.mutate(input, { onSuccess: () => setForm((f) => ({ ...f, token: '' })) })}
          >
            {save.isPending ? 'Saving…' : saved ? 'Save changes' : 'Save and start syncing'}
          </button>
          {saved && (
            <button
              type="button"
              className="btn btn-danger"
              disabled={!isAdmin || remove.isPending}
              onClick={() => window.confirm('Disconnect Fleet? Activities and metrics already imported stay.') && remove.mutate()}
            >
              Disconnect
            </button>
          )}
        </div>
      </Step>

      <Step n={4} title="Wait for the first data" open onToggle={() => {}} done={!!s.lastSyncAt}>
        {!saved ? (
          <p className="muted">Once the connection is saved, ObserveCraft reads the last 24 hours of activities and the hosts and policies, then both every minute.</p>
        ) : (
          <>
            <dl className="ci-sync">
              <dt>Fleet</dt>
              <dd>{saved.baseUrl}</dd>
              <dt>Last sync</dt>
              <dd>{s.lastSyncAt ? timeAgo(s.lastSyncAt) : 'Waiting for the first sync…'}</dd>
              <dt>Activities read up to</dt>
              <dd>{s.syncedUntil ? new Date(s.syncedUntil).toLocaleString() : '—'}</dd>
              <dt>Activities in the last sync</dt>
              <dd>{s.activitiesLastSync.toLocaleString()}</dd>
              <dt>Hosts</dt>
              <dd>{s.hostsSyncedAt ? `${s.hosts.toLocaleString()} (${timeAgo(s.hostsSyncedAt)})` : '—'}</dd>
            </dl>
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
              <Link className="btn" to={`/logs?query=${encodeURIComponent('source:fleetdm')}`}>
                View activities
              </Link>
              <Link className="btn btn-primary" to={dashboard}>
                Open the dashboard
              </Link>
            </div>
          </>
        )}
      </Step>
    </div>
  );
}

function DataCollected() {
  const dashboard = useDashboardLink();
  return (
    <div className="ci-setup">
      <h3>Dashboard</h3>
      <p className="muted">
        <Link to={dashboard}>FleetDM Overview</Link> charts the hosts (online, offline, missing and new, by platform, OS and osquery version and team, the longest unseen and the fullest disks), the
        policies (hosts failing policies, the global policies), the vulnerabilities (the most widespread CVEs, by severity, those known exploited), MDM enrollment and disk encryption, and the activity
        feed (activities by category, type and user, failed logins, streams of logins, actions on hosts and configuration changes). It appears in Dashboards once Fleet is connected.
      </p>
      <h3>Log pipeline</h3>
      <p className="muted">
        Saving the connection installs the <Link to="/logs/pipelines">FleetDM</Link> pipeline: it files each activity under <code>fleetdm.category</code>, takes a failed login&apos;s user from the
        address that was tried, and gives Fleet activities sent another way (Fleet&apos;s audit log destinations) the same attributes.
      </p>
      <h3>Activities</h3>
      <p className="muted">
        Every activity becomes a log with <code>source:fleetdm</code> and <code>service:fleetdm</code>, timed when Fleet recorded it, with the Fleet server&apos;s host name as <code>host</code>.
        Failed logins and failed installs are errors; hosts locked or wiped, encryption keys read and deletions are warnings.
      </p>
      <ul className="ci-otlp">
        {SEARCHES.map((q) => (
          <li key={q.title}>
            <Link to={`/logs?query=${encodeURIComponent(q.query)}`}>
              <b>{q.title}</b>
            </Link>{' '}
            <span className="muted">— {q.text}</span>
          </li>
        ))}
      </ul>
      <h3>Attributes</h3>
      <dl className="ci-sync">
        {ATTRIBUTES.map(([name, text]) => (
          <div key={name} style={{ display: 'contents' }}>
            <dt>
              <code>{name}</code>
            </dt>
            <dd>{text}</dd>
          </div>
        ))}
      </dl>
      <h3>Metrics</h3>
      <p className="muted">
        Every minute; every series is tagged <code>fleet_instance</code>. Host series also carry <code>fleet_host</code>, <code>fleet_host_id</code>, <code>platform</code>, <code>os_version</code>,{' '}
        <code>osquery_version</code>, <code>team</code>, <code>hardware_model</code> and <code>mdm_status</code>. Fleet&apos;s hosts aren&apos;t infrastructure hosts: they don&apos;t appear in the
        host list (a server that also runs the Datadog Agent keeps its own). Up to 10,000 hosts are read per minute; the host counts include all of them.
      </p>
      <dl className="ci-sync">
        {METRICS.map(([name, text]) => (
          <div key={name} style={{ display: 'contents' }}>
            <dt>
              <code>{name}</code>
            </dt>
            <dd>{text}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
