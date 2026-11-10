// Integrations → Google Workspace: connects a Google Workspace account with a service account that acts as an
// administrator (domain-wide delegation for the Reports API's read-only audit scope); ObserveCraft reads the selected
// applications' activities every five minutes and stores each event as a log (`source:gsuite`, `service:` the
// application), completed by the Google Workspace log pipeline, charted by the Google Workspace Overview dashboard and
// watched by the Google Workspace Cloud SIEM rules. Google needs no access to ObserveCraft.
import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { AlertTriangle, Check, ChevronRight, CircleCheck, Copy, RefreshCw, Upload } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { deleteGoogleWorkspace, googleWorkspaceStatus, saveGoogleWorkspace, syncGoogleWorkspace, testGoogleWorkspace, type GoogleWorkspaceInput } from '../../../api/googleWorkspace/client';
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

const LOGO = 'google-workspace';
const SOURCE = 'source:gsuite';
const SCOPE = 'https://www.googleapis.com/auth/admin.reports.audit.readonly';

const DESCRIPTION =
  'Google Workspace records who did what in its audit activities: logins and failed or suspicious logins, 2-step verification, Admin console changes, Drive files shared outside the domain, OAuth apps authorized, group and device changes. ObserveCraft reads them through the Admin SDK Reports API every five minutes and stores them as logs you can search, chart and alert on, with a dashboard and Cloud SIEM rules over them.';

const TABS = [
  { id: 'configure', label: 'Configuration' },
  { id: 'data', label: 'Data Collected' },
] as const;
type Tab = (typeof TABS)[number]['id'];

/** Searches over the activities, as the Data Collected tab links them. */
const SEARCHES: { title: string; text: string; query: string }[] = [
  { title: 'Failed logins', text: 'Wrong passwords (warnings), with the user, the client address and its country.', query: `${SOURCE} service:login @evt.name:login_failure` },
  { title: 'Suspicious logins', text: 'Logins Google flagged as suspicious (errors).', query: `${SOURCE} service:login @evt.name:suspicious_login` },
  { title: 'Admin console changes', text: 'Users, roles, groups and settings changed by administrators.', query: `${SOURCE} service:admin` },
  {
    title: 'Drive files shared outside',
    text: 'Files made readable by anyone with the link, public on the web, or shared with people outside the domain.',
    query: `${SOURCE} service:drive @event.parameters.visibility:(people_with_link OR public_on_the_web OR shared_externally)`,
  },
  { title: 'OAuth apps authorized', text: 'Third-party apps users granted access to their data, with the scopes.', query: `${SOURCE} service:token @evt.name:authorize` },
  { title: '2-step verification', text: 'Users enrolling in and turning off 2-step verification.', query: `${SOURCE} service:user_accounts @evt.name:(2sv_enroll OR 2sv_disable)` },
];

const ATTRIBUTES: [string, string][] = [
  ['service', 'The Reports API application (login, admin, drive, token, saml, groups, mobile, user_accounts, …)'],
  ['evt.name', 'The event (login_failure, ASSIGN_ROLE, change_document_visibility, authorize, …)'],
  ['evt.outcome', 'success or failure (logins)'],
  ['usr.email, usr.id', 'The actor and their profile ID'],
  ['network.client.ip, network.client.geoip.country.iso_code', 'The actor’s address and its country'],
  ['event.type, event.parameters.*', 'The event’s type and its parameters (doc_title, visibility, USER_EMAIL, app_name, …)'],
  ['id.time, id.uniqueQualifier, actor.*', 'The activity as Google records it'],
  ['google_workspace.category', 'authentication, authorization, administration, groups, data_access, devices, cloud or other (the log pipeline)'],
];

/** The org's Google Workspace dashboard (its id carries the org), once Google Workspace is connected. */
function useDashboardLink(): string {
  const dashboards = useIntegrationDashboards([LOGO]);
  const id = dashboards.data?.[0]?.id;
  return id ? `/dashboards/${id}` : '/dashboards?preset=integrations';
}

const statusKey = ['integrations', 'google-workspace'];

interface Form {
  key: string;
  subject: string;
  applications: string[];
}

export function GoogleWorkspaceIntegrationPage() {
  const [params, setParams] = useSearchParams();
  const tab: Tab = TABS.find((t) => t.id === params.get('tab'))?.id ?? 'configure';
  const status = useQuery({ queryKey: statusKey, queryFn: googleWorkspaceStatus, refetchInterval: (q) => (q.state.data?.connection ? 15_000 : false) });
  const installed = !!status.data?.connection;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>Google Workspace</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo={LOGO} name="Google Workspace" size={72} />
        <div>
          <h1>Google Workspace</h1>
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

function CopyText({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className="btn btn-sm"
      onClick={() =>
        navigator.clipboard?.writeText(text).then(() => {
          setDone(true);
          setTimeout(() => setDone(false), 1500);
        })
      }
    >
      {done ? <Check size={13} /> : <Copy size={13} />} {done ? 'Copied' : 'Copy'}
    </button>
  );
}

/** The key's service account and client ID, when it parses. */
function keyInfo(key: string): { email: string; clientId: string } | null {
  try {
    const k = JSON.parse(key);
    return typeof k.client_email === 'string' ? { email: k.client_email, clientId: typeof k.client_id === 'string' ? k.client_id : '' } : null;
  } catch {
    return null;
  }
}

function Configure() {
  const qc = useQueryClient();
  const refresh = () => {
    qc.invalidateQueries({ queryKey: statusKey });
    qc.invalidateQueries({ queryKey: ['dashboards'] });
    qc.invalidateQueries({ queryKey: ['integrations'] });
  };
  const status = useQuery({ queryKey: statusKey, queryFn: googleWorkspaceStatus });
  const { isAdmin } = useAccess();
  const dashboard = useDashboardLink();
  const test = useMutation({ mutationFn: (input: GoogleWorkspaceInput) => testGoogleWorkspace(input) });
  const save = useMutation({ mutationFn: (input: GoogleWorkspaceInput) => saveGoogleWorkspace(input), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteGoogleWorkspace, onSuccess: refresh });
  const sync = useMutation({ mutationFn: syncGoogleWorkspace, onSuccess: refresh });
  const [form, setForm] = useState<Form>({ key: '', subject: '', applications: [] });
  const [loadedAt, setLoadedAt] = useState<number | null>(null);
  const s = status.data;
  const saved = s?.connection ?? null;

  // Fill the form once from the saved connection (and again after it changes).
  useEffect(() => {
    const at = s?.connection?.updatedAt ?? 0;
    if (s && at !== loadedAt) {
      const c = s.connection;
      setForm({ key: '', subject: c?.subject ?? '', applications: c?.applications ?? s.available.filter((a) => a.default).map((a) => a.id) });
      setLoadedAt(at);
    }
  }, [s, loadedAt]);

  if (!s) return null;
  const set = (patch: Partial<Form>) => {
    setForm((f) => ({ ...f, ...patch }));
    test.reset();
  };
  const toggle = (id: string) => set({ applications: form.applications.includes(id) ? form.applications.filter((a) => a !== id) : [...form.applications, id] });
  const info = form.key.trim() ? keyInfo(form.key) : null;
  const input: GoogleWorkspaceInput = { serviceAccountKey: form.key.trim() || null, subject: form.subject.trim(), applications: form.applications };
  const subjectOk = /^[^\s@]+@[^\s@.]+(\.[^\s@.]+)+$/.test(input.subject);
  const keyOk = form.key.trim() ? !!info : !!saved;
  const ready = subjectOk && keyOk && form.applications.length > 0;
  const names = new Map(s.available.map((a) => [a.id, a.name]));
  const readFile = (file?: File) => file?.text().then((key) => set({ key }));

  return (
    <div className="ci-setup">
      <Step n={1} title="Create a service account and its key" open onToggle={() => {}} done={!!saved}>
        <ol className="muted">
          <li>
            In the Google Cloud console, pick (or create) a project and enable the <b>Admin SDK API</b> (APIs &amp; Services → Library).
          </li>
          <li>Create a service account (IAM &amp; Admin → Service accounts). It needs no role in the project.</li>
          <li>
            On the service account, add a key (Keys → Add key → Create new key → <b>JSON</b>) and keep the downloaded file for step 3.
          </li>
        </ol>
      </Step>

      <Step n={2} title="Let it read the audit reports (domain-wide delegation)" open onToggle={() => {}} done={!!saved}>
        <ol className="muted">
          <li>
            In the Google Admin console, open Security → Access and data control → API controls → <b>Manage domain-wide delegation</b> and add a new API client.
          </li>
          <li>
            Client ID: the service account&apos;s unique ID (its key&apos;s <code>client_id</code>
            {info?.clientId ? (
              <>
                : <code>{info.clientId}</code>
              </>
            ) : (
              ''
            )}
            ). OAuth scopes: only this one, read-only:
            <div className="aws-inline">
              <code>{SCOPE}</code> <CopyText text={SCOPE} />
            </div>
          </li>
          <li>Pick an administrator for the service account to act as: one allowed to view reports (a super administrator, or a role with the Reports privilege).</li>
        </ol>
      </Step>

      <Step n={3} title="Connect Google Workspace" open onToggle={() => {}} done={subjectOk && keyOk}>
        <label className="rt-field">
          <span>Administrator to act as</span>
          <input value={form.subject} onChange={(e) => set({ subject: e.target.value })} placeholder="admin@example.com" disabled={!isAdmin} />
          <span className="muted">The service account reads the reports as this administrator.</span>
        </label>
        <label className="rt-field">
          <span>Service account key</span>
          <textarea
            rows={4}
            value={form.key}
            onChange={(e) => set({ key: e.target.value })}
            placeholder={saved ? `Saved for ${saved.clientEmail} (leave empty to keep it)` : 'Paste the JSON key file, or upload it'}
            spellCheck={false}
            autoComplete="off"
            disabled={!isAdmin}
          />
          <span className="muted">
            {form.key.trim() ? (info ? `Service account ${info.email}.` : 'This is not a service account JSON key.') : ''} Stored encrypted and never shown again; Google is called only at its own
            token and Admin SDK endpoints.
          </span>
        </label>
        <label className="btn btn-sm" style={{ alignSelf: 'flex-start' }}>
          <Upload size={13} /> Upload the JSON key
          <input type="file" accept=".json,application/json" hidden disabled={!isAdmin} onChange={(e) => readFile(e.target.files?.[0])} />
        </label>
        <div className="rt-field">
          <span>Applications to read</span>
          <div role="group" aria-label="Applications" style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(230px, 1fr))', gap: '4px 16px' }}>
            {s.available.map((a) => (
              <label key={a.id} className="aws-inline">
                <input type="checkbox" checked={form.applications.includes(a.id)} onChange={() => toggle(a.id)} disabled={!isAdmin} />
                <span>
                  {a.name} <span className="muted">({a.id})</span>
                </span>
              </label>
            ))}
          </div>
          <span className="muted">Each application&apos;s activities become logs with its id as service (service:login, service:drive, …).</span>
        </div>
      </Step>

      <Step n={4} title="Test and save" open onToggle={() => {}} done={!!saved}>
        {!isAdmin && <p className="muted">Only organization admins can change the Google Workspace connection.</p>}
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
        {save.isSuccess && !save.isPending && <p className="muted">Saved. The first sync starts within minutes; the Google Workspace log pipeline and dashboard are installed.</p>}
        <div className="rt-create-actions">
          <button type="button" className="btn" disabled={!isAdmin || !ready || test.isPending} onClick={() => test.mutate(input)}>
            {test.isPending ? 'Testing…' : 'Test connection'}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!isAdmin || !ready || save.isPending}
            onClick={() => save.mutate(input, { onSuccess: () => setForm((f) => ({ ...f, key: '' })) })}
          >
            {save.isPending ? 'Saving…' : saved ? 'Save changes' : 'Save and start syncing'}
          </button>
          {saved && (
            <button
              type="button"
              className="btn btn-danger"
              disabled={!isAdmin || remove.isPending}
              onClick={() => window.confirm('Disconnect Google Workspace? Activities already imported stay.') && remove.mutate()}
            >
              Disconnect
            </button>
          )}
        </div>
      </Step>

      <Step n={5} title="Wait for the first activities" open onToggle={() => {}} done={s.syncs.some((x) => !!x.lastSyncAt)}>
        {!saved ? (
          <p className="muted">
            Once the connection is saved, ObserveCraft reads the last 24 hours of activities, then new ones every five minutes. Google delivers some applications&apos; activities (login, token,
            groups, Drive) up to hours late: ObserveCraft reads the last four hours again every half hour, so they arrive late too, but once.
          </p>
        ) : (
          <>
            <dl className="ci-sync">
              <dt>Service account</dt>
              <dd>{saved.clientEmail}</dd>
              <dt>Acting as</dt>
              <dd>{saved.subject}</dd>
            </dl>
            <table className="im-table">
              <thead>
                <tr>
                  <th>Application</th>
                  <th>Last sync</th>
                  <th>Read up to</th>
                  <th>Events in the last sync</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {s.syncs.map((x) => (
                  <tr key={x.application}>
                    <td>
                      <Link to={`/logs?query=${encodeURIComponent(`${SOURCE} service:${x.application}`)}`}>{names.get(x.application) ?? x.application}</Link>
                    </td>
                    <td>{x.lastSyncAt ? timeAgo(x.lastSyncAt) : 'Waiting for the first sync…'}</td>
                    <td>{x.syncedUntil ? new Date(x.syncedUntil).toLocaleString() : '—'}</td>
                    <td>
                      {x.eventsLastSync.toLocaleString()}
                      {x.catchingUp && <span className="muted"> (catching up on a backlog)</span>}
                    </td>
                    <td>
                      {x.error ? (
                        <span className="form-error" title={x.error.message}>
                          <AlertTriangle size={13} /> {x.error.message} <span className="muted">({timeAgo(x.error.at)})</span>
                        </span>
                      ) : (
                        <span className="muted">OK</span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="rt-create-actions">
              <button type="button" className="btn" disabled={!isAdmin || sync.isPending} onClick={() => sync.mutate()}>
                <RefreshCw size={14} /> {sync.isPending ? 'Syncing…' : 'Sync now'}
              </button>
              <Link className="btn" to={`/logs?query=${encodeURIComponent(SOURCE)}`}>
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
        <Link to={dashboard}>Google Workspace Overview</Link> charts the activities by application and category, logins and failed or suspicious logins with their users, addresses and countries,
        2-step verification, Admin console changes and roles granted, Drive files shared outside the domain, OAuth apps authorized, group and device changes, and the Cloud SIEM signals of the Google
        Workspace rules. It appears in Dashboards once Google Workspace is connected.
      </p>
      <h3>Log pipeline</h3>
      <p className="muted">
        Saving the connection installs the <Link to="/logs/pipelines">Google Workspace</Link> pipeline: it files each activity under <code>google_workspace.category</code>, sets suspicious logins and
        disabled accounts as errors and failed logins and blocked actions as warnings, and gives Reports API activities sent another way (a log shipper) the standard attributes.
      </p>
      <h3>Cloud SIEM</h3>
      <p className="muted">
        The <Link to="/security/siem/content-packs">Google Workspace</Link> content pack&apos;s detection rules raise signals when logins fail again and again for one user, Google flags a login as
        suspicious, a user is granted an administrator role, a Drive file is shared publicly and 2-step verification is turned off.
      </p>
      <h3>Events</h3>
      <p className="muted">
        Every event of an activity becomes a log with <code>{SOURCE}</code> and the application as <code>service</code>, as Datadog&apos;s Google Workspace integration sends them, timed when Google
        recorded it.
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
