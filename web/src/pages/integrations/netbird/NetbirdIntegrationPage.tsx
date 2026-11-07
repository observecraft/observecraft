// Integrations → NetBird: connects a NetBird account (NetBird Cloud or a self-hosted management server) with a
// service user's personal access token; ObserveCraft polls its audit events and peers every minute and stores
// them as logs (`source:netbird`) and `netbird.peer.*` metrics, charted by the NetBird Overview dashboard.
// NetBird needs no access to ObserveCraft.
import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { AlertTriangle, Check, ChevronRight, CircleCheck, RefreshCw } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { deleteNetbird, netbirdStatus, saveNetbird, syncNetbird, testNetbird, type NetbirdInput } from '../../../api/netbird/client';
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
  'NetBird connects devices, servers and cloud resources over a WireGuard-based overlay network. ObserveCraft reads an account’s management API every minute, from NetBird Cloud or a self-hosted server: the audit log (peers added and removed, policies, routes, networks, DNS, setup keys, users and logins) as logs, and every peer’s state (connected, last seen, login expired, OS and client version) as metrics, with a dashboard over both.';

const CLOUD_URL = 'https://api.netbird.io';

/** The org's NetBird dashboard (its id carries the org), once NetBird is connected. */
function useDashboardLink(): string {
  const dashboards = useIntegrationDashboards(['netbird']);
  const id = dashboards.data?.[0]?.id;
  return id ? `/dashboards/${id}` : '/dashboards?preset=integrations';
}

const TABS = [
  { id: 'configure', label: 'Configuration' },
  { id: 'data', label: 'Data Collected' },
] as const;
type Tab = (typeof TABS)[number]['id'];

/** Searches over the audit events, as the Data Collected tab links them. */
const SEARCHES: { title: string; text: string; query: string }[] = [
  { title: 'Peers added and removed', text: 'Peers registered by a user or a setup key, deleted, approved or whose login expired.', query: 'source:netbird @netbird.category:peer' },
  {
    title: 'Access changes',
    text: 'Policies, routes, networks, nameservers, DNS and posture checks created, updated or deleted.',
    query: 'source:netbird @netbird.category:(policy OR route OR network OR nameserver OR dns OR posture)',
  },
  {
    title: 'Users and tokens',
    text: 'Users invited, blocked or whose role changed; service users and personal access tokens.',
    query: 'source:netbird @netbird.category:(user OR service OR personal)',
  },
  { title: 'Setup keys', text: 'Setup keys created, revoked and overused (errors).', query: 'source:netbird @netbird.category:setupkey' },
  { title: 'Logins', text: 'Logins to the NetBird dashboard and peers logged in by their users.', query: 'source:netbird @evt.name:(dashboard.login OR user.peer.login)' },
  { title: 'Warnings and errors', text: 'Blocked users, expired logins, revoked keys and overused setup keys.', query: 'source:netbird status:(warn OR error)' },
];

const ATTRIBUTES: [string, string][] = [
  ['evt.name', 'NetBird’s activity code (peer.user.add, policy.update, setupkey.overuse, …)'],
  ['usr.id, usr.name, usr.email', 'Who did it; absent for NetBird’s own events (expirations, overuse)'],
  ['netbird.category', 'The code’s first segment: peer, user, policy, route, setupkey, group, account, dns, network, …'],
  ['netbird.activity, netbird.target, netbird.target_id', 'NetBird’s description, the name and the id of what the event touched'],
  ['netbird.meta', 'The rest of the event’s metadata, as NetBird records it'],
];

const METRICS: [string, string][] = [
  ['netbird.peer.count', '1 per peer: summed, the peers (by os, netbird_version, country_code)'],
  ['netbird.peer.connected', '1 while the peer is connected, else 0'],
  ['netbird.peer.last_seen_age', 'Seconds since the peer was last seen (0 while connected)'],
  ['netbird.peer.login_expired', '1 when the peer’s login expired'],
  ['netbird.peer.approval_required', '1 while the peer waits for an admin’s approval'],
  ['netbird.peer.ssh_enabled', '1 when the peer runs NetBird’s SSH server'],
  ['netbird.peer.accessible_peers', 'The peers its policies let it reach'],
];

const statusKey = ['integrations', 'netbird'];

interface Form {
  baseUrl: string;
  token: string;
}

export function NetbirdIntegrationPage() {
  const [params, setParams] = useSearchParams();
  const tab: Tab = TABS.find((t) => t.id === params.get('tab'))?.id ?? 'configure';
  const status = useQuery({ queryKey: statusKey, queryFn: netbirdStatus, refetchInterval: (q) => (q.state.data?.connection ? 15_000 : false) });
  const installed = !!status.data?.connection;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>NetBird</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo="netbird" name="NetBird" size={72} />
        <div>
          <h1>NetBird</h1>
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
  const refresh = () => qc.invalidateQueries({ queryKey: statusKey });
  const status = useQuery({ queryKey: statusKey, queryFn: netbirdStatus });
  const { isAdmin } = useAccess();
  const test = useMutation({ mutationFn: (input: NetbirdInput) => testNetbird(input) });
  const save = useMutation({ mutationFn: (input: NetbirdInput) => saveNetbird(input), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteNetbird, onSuccess: refresh });
  const sync = useMutation({ mutationFn: syncNetbird, onSuccess: refresh });
  const dashboard = useDashboardLink();
  const [form, setForm] = useState<Form>({ baseUrl: CLOUD_URL, token: '' });
  const [loadedAt, setLoadedAt] = useState<number | null>(null);
  const s = status.data;
  const saved = s?.connection ?? null;

  // Fill the form once from the saved connection (and again after it changes).
  useEffect(() => {
    const at = s?.connection?.updatedAt ?? 0;
    if (s && at !== loadedAt) {
      setForm({ baseUrl: s.connection?.baseUrl ?? CLOUD_URL, token: '' });
      setLoadedAt(at);
    }
  }, [s, loadedAt]);

  if (!s) return null;
  const set = (patch: Partial<Form>) => {
    setForm((f) => ({ ...f, ...patch }));
    test.reset();
  };
  const baseUrl = form.baseUrl
    .trim()
    .replace(/\/+$/, '')
    .replace(/\/api$/, '');
  const input: NetbirdInput = { baseUrl, token: form.token.trim() || null };
  const urlOk = /^https?:\/\/[^\s/]+/.test(baseUrl);
  const tokenOk = !!input.token || saved?.baseUrl === baseUrl;
  const ready = urlOk && tokenOk;

  return (
    <div className="ci-setup">
      <Step n={1} title="Create a service user and a token in NetBird" open onToggle={() => {}} done={!!saved}>
        <ol className="muted">
          <li>
            In the NetBird dashboard, open <b>Team → Service Users</b> and create one, e.g. <code>observecraft-reader</code>, with the <b>Auditor</b> role (read-only; <b>Admin</b> works too). A
            regular user can&apos;t read the audit log.
          </li>
          <li>
            Open the service user and <b>Create Token</b> (an expiration of up to a year); copy it, NetBird shows it once.
          </li>
        </ol>
      </Step>

      <Step n={2} title="Connect the account" open onToggle={() => {}} done={urlOk && tokenOk}>
        <label className="rt-field">
          <span>Management URL</span>
          <input value={form.baseUrl} onChange={(e) => set({ baseUrl: e.target.value })} placeholder={CLOUD_URL} disabled={!isAdmin} />
          <span className="muted">
            <code>{CLOUD_URL}</code> for NetBird Cloud; your management server&apos;s URL when self-hosted (the one the dashboard calls, e.g. <code>https://netbird.example.com</code>). ObserveCraft
            calls its <code>/api</code> from its servers over HTTPS.
          </span>
        </label>
        <label className="rt-field">
          <span>Personal access token</span>
          <input
            type="password"
            value={form.token}
            onChange={(e) => set({ token: e.target.value })}
            autoComplete="new-password"
            placeholder={saved?.baseUrl === baseUrl ? 'Saved (leave empty to keep it)' : 'nbp_…'}
            disabled={!isAdmin}
          />
          <span className="muted">Stored encrypted and never shown again.</span>
        </label>
      </Step>

      <Step n={3} title="Test and save" open onToggle={() => {}} done={!!saved}>
        {!isAdmin && <p className="muted">Only organization admins can change the NetBird connection.</p>}
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
        {save.isSuccess && !save.isPending && <p className="muted">Saved. The first sync starts within a minute.</p>}
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
              onClick={() => window.confirm('Disconnect NetBird? Events and metrics already imported stay.') && remove.mutate()}
            >
              Disconnect
            </button>
          )}
        </div>
      </Step>

      <Step n={4} title="Wait for the first data" open onToggle={() => {}} done={!!s.lastSyncAt}>
        {!saved ? (
          <p className="muted">Once the connection is saved, ObserveCraft reads the last 24 hours of audit events and the peers, then both every minute.</p>
        ) : (
          <>
            <dl className="ci-sync">
              <dt>NetBird</dt>
              <dd>{saved.baseUrl}</dd>
              <dt>Last sync</dt>
              <dd>{s.lastSyncAt ? timeAgo(s.lastSyncAt) : 'Waiting for the first sync…'}</dd>
              <dt>Events read up to</dt>
              <dd>{s.syncedUntil ? new Date(s.syncedUntil).toLocaleString() : '—'}</dd>
              <dt>Events in the last sync</dt>
              <dd>{s.eventsLastSync.toLocaleString()}</dd>
              <dt>Peers</dt>
              <dd>{s.peersSyncedAt ? `${s.peers.toLocaleString()} (${timeAgo(s.peersSyncedAt)})` : '—'}</dd>
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
              <Link className="btn" to={`/logs?query=${encodeURIComponent('source:netbird')}`}>
                View events
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
        <Link to={dashboard}>NetBird Overview</Link> charts the peers (connected, disconnected, expired logins, waiting for approval, by OS, client version and country, the longest unseen) and the
        audit log (events by category, top activities and users, streams of peer, access and user changes). It appears in Dashboards once NetBird is connected.
      </p>
      <h3>Audit events</h3>
      <p className="muted">
        Every audit event becomes a log with <code>source:netbird</code> and <code>service:netbird</code>, timed when NetBird recorded it, with the management server&apos;s host name as{' '}
        <code>host</code>. Overused setup keys are errors; blocked users, expired logins and revoked keys are warnings.
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
        One series per peer, every minute, tagged <code>netbird_instance</code>, <code>peer</code>, <code>peer_id</code>, <code>os</code>, <code>netbird_version</code> and, when NetBird located it,{' '}
        <code>country_code</code> and <code>city</code>. Peers aren&apos;t hosts: they don&apos;t appear in the infrastructure list.
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
