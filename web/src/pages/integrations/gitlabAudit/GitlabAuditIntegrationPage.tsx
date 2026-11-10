// Integrations → GitLab Audit Events: connects a GitLab instance (GitLab.com or self-managed, Premium or Ultimate) with
// an access token, for the whole instance or a group or project; ObserveCraft polls its audit events every minute and
// stores them as logs (`source:gitlab-audit-events`), completed by the GitLab Audit Events log pipeline, charted by
// the GitLab Audit Events Overview dashboard and watched by the GitLab Cloud SIEM rules. GitLab needs no access to
// ObserveCraft.
import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { AlertTriangle, Check, ChevronRight, CircleCheck, RefreshCw } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { deleteGitlabAudit, gitlabAuditStatus, saveGitlabAudit, syncGitlabAudit, testGitlabAudit, type GitlabAuditInput } from '../../../api/gitlabAudit/client';
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

const LOGO = 'gitlab-audit-events';
const SOURCE = 'source:gitlab-audit-events';

const DESCRIPTION =
  'GitLab records who did what in its audit events: failed logins, access tokens, members and their roles, OAuth applications authorized, merge request approvals and merges, project, group and CI/CD settings. ObserveCraft reads them through GitLab’s API every minute and stores them as logs you can search, chart and alert on, with a dashboard and Cloud SIEM rules over them.';

const TABS = [
  { id: 'configure', label: 'Configuration' },
  { id: 'data', label: 'Data Collected' },
] as const;
type Tab = (typeof TABS)[number]['id'];

type Scope = 'instance' | 'group' | 'project';
const SCOPES: { id: Scope; label: string; text: string }[] = [
  { id: 'instance', label: 'The whole instance', text: 'Every audit event of a self-managed instance (an administrator’s token).' },
  { id: 'group', label: 'A group', text: 'The events of a group and its projects (a token of one of its Owners, or a group access token).' },
  { id: 'project', label: 'A project', text: 'The events of one project (a Maintainer’s token, or a project access token).' },
];

/** Searches over the events, as the Data Collected tab links them. */
const SEARCHES: { title: string; text: string; query: string }[] = [
  { title: 'Failed logins', text: 'Wrong passwords and one-time codes (errors), with the user tried and the client address.', query: `${SOURCE} @evt.name:*login_failed*` },
  { title: 'Access tokens', text: 'Personal, project, group and deploy tokens created, rotated and revoked.', query: `${SOURCE} @gitlab.category:token` },
  { title: 'Members and authorizations', text: 'Members added, their roles changed or removed, OAuth applications authorized.', query: `${SOURCE} @gitlab.category:(membership OR authorization)` },
  { title: 'Merge requests', text: 'Merge requests created, approved and merged, approval rules changed.', query: `${SOURCE} @gitlab.category:merge_request` },
  { title: 'Projects and groups', text: 'Projects and groups created, deleted, exported or made more visible.', query: `${SOURCE} @gitlab.category:(project OR group)` },
  { title: 'CI/CD and repositories', text: 'CI/CD variables and runners, protected branches and push rules.', query: `${SOURCE} @gitlab.category:(ci_cd OR repository)` },
];

const ATTRIBUTES: [string, string][] = [
  ['evt.name', 'GitLab’s event type (personal_access_token_created, merge_request_merged, …)'],
  ['evt.outcome', 'success or failure (failed logins)'],
  ['usr.name, usr.email, usr.id', 'The author (for a failed login: the user that was tried)'],
  ['network.client.ip', 'The author’s address'],
  ['gitlab.entity_type, gitlab.entity_id, gitlab.entity_path', 'The user, group or project the event belongs to'],
  ['gitlab.target_type, gitlab.target_id, gitlab.target_details', 'What it changed (a member, a token, a merge request, a setting)'],
  ['gitlab.details', 'The rest of GitLab’s details (change, from, to, add, as, …)'],
  ['gitlab.category', 'authentication, token, merge_request, membership, authorization, user_management, ci_cd, repository, project, group, compliance or other (the log pipeline)'],
];

/** The org's GitLab Audit Events dashboard (its id carries the org), once GitLab is connected. */
function useDashboardLink(): string {
  const dashboards = useIntegrationDashboards([LOGO]);
  const id = dashboards.data?.[0]?.id;
  return id ? `/dashboards/${id}` : '/dashboards?preset=integrations';
}

const statusKey = ['integrations', 'gitlab-audit-events'];

interface Form {
  baseUrl: string;
  token: string;
  scope: Scope;
  path: string;
}

export function GitlabAuditIntegrationPage() {
  const [params, setParams] = useSearchParams();
  const tab: Tab = TABS.find((t) => t.id === params.get('tab'))?.id ?? 'configure';
  const status = useQuery({ queryKey: statusKey, queryFn: gitlabAuditStatus, refetchInterval: (q) => (q.state.data?.connection ? 15_000 : false) });
  const installed = !!status.data?.connection;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>GitLab Audit Events</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo={LOGO} name="GitLab Audit Events" size={72} />
        <div>
          <h1>GitLab Audit Events</h1>
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
  const status = useQuery({ queryKey: statusKey, queryFn: gitlabAuditStatus });
  const { isAdmin } = useAccess();
  const dashboard = useDashboardLink();
  const test = useMutation({ mutationFn: (input: GitlabAuditInput) => testGitlabAudit(input) });
  const save = useMutation({ mutationFn: (input: GitlabAuditInput) => saveGitlabAudit(input), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteGitlabAudit, onSuccess: refresh });
  const sync = useMutation({ mutationFn: syncGitlabAudit, onSuccess: refresh });
  const [form, setForm] = useState<Form>({ baseUrl: '', token: '', scope: 'group', path: '' });
  const [loadedAt, setLoadedAt] = useState<number | null>(null);
  const s = status.data;
  const saved = s?.connection ?? null;

  // Fill the form once from the saved connection (and again after it changes).
  useEffect(() => {
    const at = s?.connection?.updatedAt ?? 0;
    if (s && at !== loadedAt) {
      const c = s.connection;
      setForm({ baseUrl: c?.baseUrl ?? '', token: '', scope: (c?.scope as Scope | undefined) ?? 'group', path: c?.path ?? '' });
      setLoadedAt(at);
    }
  }, [s, loadedAt]);

  if (!s) return null;
  const set = (patch: Partial<Form>) => {
    setForm((f) => ({ ...f, ...patch }));
    test.reset();
  };
  const baseUrl = form.baseUrl.trim().replace(/\/+$/, '') || 'https://gitlab.com';
  const path = form.scope === 'instance' ? '' : form.path.trim().replace(/^\/+|\/+$/g, '');
  const input: GitlabAuditInput = { baseUrl, token: form.token.trim() || null, scope: form.scope, path };
  const urlOk = /^https?:\/\/[^\s/]+/.test(baseUrl);
  const pathOk = form.scope === 'instance' || /^[\w.-]+(\/[\w.-]+)*$/.test(path);
  const tokenOk = !!input.token || saved?.baseUrl === baseUrl;
  const ready = urlOk && pathOk && tokenOk;
  const scopeLabel = (c: { scope: string; path: string }) => (c.scope === 'instance' ? 'The whole instance' : `${c.scope === 'group' ? 'Group' : 'Project'} ${c.path}`);

  return (
    <div className="ci-setup">
      <Step n={1} title="Choose whose audit events to read" open onToggle={() => {}} done={!!saved}>
        <p className="muted">
          GitLab keeps audit events on Premium and Ultimate. On GitLab.com, read a top-level group&apos;s; on a self-managed instance, an administrator can read the whole instance&apos;s.
        </p>
        <div className="ci-otlp" role="radiogroup" aria-label="Scope">
          {SCOPES.map((o) => (
            <label key={o.id} className="aws-inline">
              <input type="radio" name="gitlab-audit-scope" checked={form.scope === o.id} onChange={() => set({ scope: o.id })} disabled={!isAdmin} />
              <span>
                <b>{o.label}</b> <span className="muted">— {o.text}</span>
              </span>
            </label>
          ))}
        </div>
        {form.scope !== 'instance' && (
          <label className="rt-field">
            <span>{form.scope === 'group' ? 'Group path' : 'Project path'}</span>
            <input value={form.path} onChange={(e) => set({ path: e.target.value })} placeholder={form.scope === 'group' ? 'example-org' : 'example-org/storefront-api'} disabled={!isAdmin} />
            <span className="muted">As in its URL, or its numeric ID.</span>
          </label>
        )}
      </Step>

      <Step n={2} title="Create an access token" open onToggle={() => {}} done={!!saved}>
        <ol className="muted">
          <li>
            In GitLab, create a token with the <code>read_api</code> scope: a <b>personal access token</b> (User settings → Access tokens) of an{' '}
            {form.scope === 'instance' ? 'administrator' : form.scope === 'group' ? 'Owner of the group' : 'Maintainer of the project'}
            {form.scope === 'instance' ? (
              ''
            ) : (
              <>
                , or a {form.scope} access token ({form.scope === 'group' ? 'the group' : 'the project'} → Settings → Access tokens) with that role
              </>
            )}
            .
          </li>
          <li>Give it an expiry date you will remember to renew: ObserveCraft reports a refused token on this page.</li>
        </ol>
      </Step>

      <Step n={3} title="Connect GitLab" open onToggle={() => {}} done={urlOk && tokenOk}>
        <label className="rt-field">
          <span>GitLab URL</span>
          <input value={form.baseUrl} onChange={(e) => set({ baseUrl: e.target.value })} placeholder="https://gitlab.com" disabled={!isAdmin} />
          <span className="muted">GitLab.com or your instance. ObserveCraft calls its API (/api/v4) from its servers over HTTPS.</span>
        </label>
        <label className="rt-field">
          <span>Access token</span>
          <input
            type="password"
            value={form.token}
            onChange={(e) => set({ token: e.target.value })}
            autoComplete="new-password"
            placeholder={saved?.baseUrl === baseUrl ? 'Saved (leave empty to keep it)' : 'glpat-…'}
            disabled={!isAdmin}
          />
          <span className="muted">Stored encrypted and never shown again.</span>
        </label>
      </Step>

      <Step n={4} title="Test and save" open onToggle={() => {}} done={!!saved}>
        {!isAdmin && <p className="muted">Only organization admins can change the GitLab connection.</p>}
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
        {save.isSuccess && !save.isPending && <p className="muted">Saved. The first sync starts within a minute; the GitLab Audit Events log pipeline and dashboard are installed.</p>}
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
              onClick={() => window.confirm('Disconnect GitLab? Audit events already imported stay.') && remove.mutate()}
            >
              Disconnect
            </button>
          )}
        </div>
      </Step>

      <Step n={5} title="Wait for the first events" open onToggle={() => {}} done={!!s.lastSyncAt}>
        {!saved ? (
          <p className="muted">Once the connection is saved, ObserveCraft reads the last 24 hours of audit events, then new ones every minute.</p>
        ) : (
          <>
            <dl className="ci-sync">
              <dt>GitLab</dt>
              <dd>{saved.baseUrl}</dd>
              <dt>Audit events of</dt>
              <dd>{scopeLabel(saved)}</dd>
              <dt>Last sync</dt>
              <dd>{s.lastSyncAt ? timeAgo(s.lastSyncAt) : 'Waiting for the first sync…'}</dd>
              <dt>Events read up to</dt>
              <dd>{s.syncedUntil ? new Date(s.syncedUntil).toLocaleString() : '—'}</dd>
              <dt>Events in the last sync</dt>
              <dd>
                {s.eventsLastSync.toLocaleString()}
                {s.catchingUp && <span className="muted"> (catching up on a backlog)</span>}
              </dd>
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
              <Link className="btn" to={`/logs?query=${encodeURIComponent(SOURCE)}`}>
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
        <Link to={dashboard}>GitLab Audit Events Overview</Link> charts the audit events by category, the most active users and addresses, failed logins, members added and OAuth applications
        authorized, merge requests merged and approved and approval rules changed, project activity, access tokens created, rotated and revoked, and the Cloud SIEM signals of the GitLab rules. It
        appears in Dashboards once GitLab is connected.
      </p>
      <h3>Log pipeline</h3>
      <p className="muted">
        Saving the connection installs the <Link to="/logs/pipelines">GitLab Audit Events</Link> pipeline: it files each event under <code>gitlab.category</code>, sets failed logins as errors and
        locked or blocked accounts as warnings, and gives GitLab audit events sent another way (a streaming destination, a log shipper) the same attributes.
      </p>
      <h3>Cloud SIEM</h3>
      <p className="muted">
        The <Link to="/security/siem/content-packs">GitLab</Link> content pack&apos;s detection rules raise signals when logins fail again and again for one user and when a project is made public.
      </p>
      <h3>Events</h3>
      <p className="muted">
        Every audit event becomes a log with <code>{SOURCE}</code> and <code>service:gitlab</code>, timed when GitLab recorded it, with the instance&apos;s host name as <code>host</code>.
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
