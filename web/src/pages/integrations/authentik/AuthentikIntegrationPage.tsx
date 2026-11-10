// Integrations → authentik: connects an authentik instance with an API token; ObserveCraft polls its events
// (logins, access requests, users and objects changed, policy and system errors) every minute and stores them
// as logs (`source:authentik`), completed by the authentik log pipeline and charted by the authentik Overview
// dashboard. authentik needs no access to ObserveCraft.
import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { AlertTriangle, Check, ChevronRight, CircleCheck, RefreshCw } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { authentikStatus, deleteAuthentik, saveAuthentik, syncAuthentik, testAuthentik, type AuthentikInput } from '../../../api/authentik/client';
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
  'authentik is an open-source identity provider (SSO, MFA, application access). ObserveCraft reads its event log through the API every minute: logins and failed logins, application authorizations, access requests, users, groups and policies changed, and the instance’s errors, stored as logs you can search, chart and alert on, with a dashboard over them.';

const TABS = [
  { id: 'configure', label: 'Configuration' },
  { id: 'data', label: 'Data Collected' },
] as const;
type Tab = (typeof TABS)[number]['id'];

/** Searches over the events, as the Data Collected tab links them. */
const SEARCHES: { title: string; text: string; query: string }[] = [
  { title: 'Logins', text: 'Successful logins, with the method, client IP, country and user agent.', query: 'source:authentik @evt.name:login' },
  { title: 'Failed logins', text: 'Wrong passwords, refused MFA and unknown users (errors).', query: 'source:authentik @evt.name:login_failed' },
  { title: 'Application access', text: 'Users authorized to an application (OAuth2, SAML, proxy).', query: 'source:authentik @evt.name:authorize_application' },
  { title: 'Access requests', text: 'Requests created, approved, denied (warnings) and revoked.', query: 'source:authentik @evt.name:access_request_*' },
  {
    title: 'Users and objects changed',
    text: 'Users, groups, applications, providers and policies created, updated or deleted.',
    query: 'source:authentik @evt.name:(model_created OR model_updated OR model_deleted)',
  },
  { title: 'Errors', text: 'Policy and property mapping exceptions, configuration errors and suspicious requests.', query: 'source:authentik status:error' },
];

const ATTRIBUTES: [string, string][] = [
  ['evt.name', 'The event’s action (login, login_failed, model_created, …)'],
  ['evt.outcome', 'success or failure'],
  ['usr.name, usr.email, usr.id', 'The user the event is about'],
  ['network.client.ip', 'The client’s address, with its geoip country and city and its ASN'],
  ['http.method, http.url_details.path, http.useragent', 'The request that caused the event'],
  ['authentik.app, authentik.application, authentik.brand', 'The authentik module, the application and the brand involved'],
  [
    'authentik.category',
    'authentication, mfa, password, impersonation, application_access, access_request, suspicious, policy, configuration_change, user_lifecycle, system or other (the log pipeline)',
  ],
  ['authentik.context', 'The rest of the event’s context, as authentik records it'],
];

/** The org's authentik dashboard (its id carries the org), once authentik is connected. */
function useDashboardLink(): string {
  const dashboards = useIntegrationDashboards(['authentik']);
  const id = dashboards.data?.[0]?.id;
  return id ? `/dashboards/${id}` : '/dashboards?preset=integrations';
}

const statusKey = ['integrations', 'authentik'];

interface Form {
  baseUrl: string;
  token: string;
}

export function AuthentikIntegrationPage() {
  const [params, setParams] = useSearchParams();
  const tab: Tab = TABS.find((t) => t.id === params.get('tab'))?.id ?? 'configure';
  const status = useQuery({ queryKey: statusKey, queryFn: authentikStatus, refetchInterval: (q) => (q.state.data?.connection ? 15_000 : false) });
  const installed = !!status.data?.connection;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>authentik</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo="authentik" name="authentik" size={72} />
        <div>
          <h1>authentik</h1>
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
    qc.invalidateQueries({ queryKey: ['integrations'] });
  };
  const status = useQuery({ queryKey: statusKey, queryFn: authentikStatus });
  const { isAdmin } = useAccess();
  const dashboard = useDashboardLink();
  const test = useMutation({ mutationFn: (input: AuthentikInput) => testAuthentik(input) });
  const save = useMutation({ mutationFn: (input: AuthentikInput) => saveAuthentik(input), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteAuthentik, onSuccess: refresh });
  const sync = useMutation({ mutationFn: syncAuthentik, onSuccess: refresh });
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
  const input: AuthentikInput = { baseUrl, token: form.token.trim() || null };
  const urlOk = /^https?:\/\/[^\s/]+/.test(baseUrl);
  const tokenOk = !!input.token || saved?.baseUrl === baseUrl;
  const ready = urlOk && tokenOk;

  return (
    <div className="ci-setup">
      <Step n={1} title="Create a read-only API token in authentik" open onToggle={() => {}} done={!!saved}>
        <ol className="muted">
          <li>
            In authentik&apos;s admin interface, create a <b>service account</b> (Directory → Users → Create Service account), e.g. <code>observecraft-reader</code>.
          </li>
          <li>
            Give it a role with the <code>authentik_events.view_event</code> permission (Directory → Roles), so it can read the event log and nothing else.
          </li>
          <li>
            Create an <b>API token</b> for it (Directory → Tokens and App passwords → Create, intent <i>API</i>) and copy its key.
          </li>
        </ol>
      </Step>

      <Step n={2} title="Connect the instance" open onToggle={() => {}} done={urlOk && tokenOk}>
        <label className="rt-field">
          <span>authentik URL</span>
          <input value={form.baseUrl} onChange={(e) => set({ baseUrl: e.target.value })} placeholder="https://auth.example.com" disabled={!isAdmin} />
          <span className="muted">ObserveCraft calls the instance&apos;s API (/api/v3) from its servers over HTTPS.</span>
        </label>
        <label className="rt-field">
          <span>API token</span>
          <input
            type="password"
            value={form.token}
            onChange={(e) => set({ token: e.target.value })}
            autoComplete="new-password"
            placeholder={saved?.baseUrl === baseUrl ? 'Saved (leave empty to keep it)' : 'The token’s key'}
            disabled={!isAdmin}
          />
          <span className="muted">Stored encrypted and never shown again.</span>
        </label>
      </Step>

      <Step n={3} title="Test and save" open onToggle={() => {}} done={!!saved}>
        {!isAdmin && <p className="muted">Only organization admins can change the authentik connection.</p>}
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
        {save.isSuccess && !save.isPending && <p className="muted">Saved. The first sync starts within a minute; the authentik log pipeline and dashboard are installed.</p>}
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
              onClick={() => window.confirm('Disconnect authentik? Events already imported stay.') && remove.mutate()}
            >
              Disconnect
            </button>
          )}
        </div>
      </Step>

      <Step n={4} title="Wait for the first events" open onToggle={() => {}} done={!!s.lastSyncAt}>
        {!saved ? (
          <p className="muted">Once the connection is saved, ObserveCraft reads the last 24 hours of events, then new ones every minute.</p>
        ) : (
          <>
            <dl className="ci-sync">
              <dt>authentik</dt>
              <dd>{saved.baseUrl}</dd>
              <dt>Last sync</dt>
              <dd>{s.lastSyncAt ? timeAgo(s.lastSyncAt) : 'Waiting for the first sync…'}</dd>
              <dt>Events read up to</dt>
              <dd>{s.syncedUntil ? new Date(s.syncedUntil).toLocaleString() : '—'}</dd>
              <dt>Events in the last sync</dt>
              <dd>{s.eventsLastSync.toLocaleString()}</dd>
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
              <Link className="btn" to={`/logs?query=${encodeURIComponent('source:authentik')}`}>
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
        <Link to={dashboard}>authentik Overview</Link> charts logins (over time, by method, country, city and network, the users logging in most), failed logins (by address, user, stage and country),
        security events (suspicious requests, impersonation, MFA authenticators enrolled or removed, passwords changed), the applications authorized and access requests, administrators&apos; changes
        and the instance&apos;s errors. It appears in Dashboards once authentik is connected.
      </p>
      <h3>Log pipeline</h3>
      <p className="muted">
        Saving the connection installs the <Link to="/logs/pipelines">authentik</Link> pipeline: it files each event under <code>authentik.category</code>, takes a failed login&apos;s user from the
        name that was tried, and gives authentik events sent another way (a notification webhook, a log shipper) the same attributes.
      </p>
      <h3>Events</h3>
      <p className="muted">
        Every authentik event becomes a log with <code>source:authentik</code> and <code>service:authentik</code>, timed when authentik recorded it, with the instance&apos;s host name as{' '}
        <code>host</code>. Failures and exceptions are errors; denied access requests and impersonation are warnings.
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
    </div>
  );
}
