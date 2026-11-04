// Software Delivery → Settings → GitLab CI: steps 2-5 of the CI provider setup. ObserveCraft polls GitLab's
// API with a read_api token (GitLab.com or a self-managed instance; GitLab needs no access to ObserveCraft)
// and imports each finished pipeline as a CI/CD trace.
import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { AlertTriangle, Check, CircleCheck, RefreshCw } from 'lucide-react';
import type { GitlabCiInput, GitlabCiStatus } from '../../api/ci/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Step } from '../integrations/Step';
import { useAccess } from '../settings/queries';
import { fmtAgo } from './common';
import { useGitlabCi, useGitlabCiMutations } from './queries';

const SAAS_URL = 'https://gitlab.com';

type Install = 'saas' | 'self-managed';

const INSTALLS: { id: Install; title: string; text: string }[] = [
  { id: 'saas', title: 'GitLab.com (SaaS)', text: 'Projects hosted on gitlab.com.' },
  { id: 'self-managed', title: 'Self-managed', text: 'Your own GitLab instance (Community or Enterprise Edition), reachable from ObserveCraft over HTTPS.' },
];

interface Form {
  install: Install;
  baseUrl: string;
  token: string;
  groupPath: string;
  includeSubgroups: boolean;
  projects: string;
}

function formOf(s: GitlabCiStatus | undefined): Form {
  const c = s?.connection;
  if (!c) return { install: 'saas', baseUrl: '', token: '', groupPath: '', includeSubgroups: true, projects: '' };
  const saas = c.baseUrl === SAAS_URL;
  return { install: saas ? 'saas' : 'self-managed', baseUrl: saas ? '' : c.baseUrl, token: '', groupPath: c.groupPath, includeSubgroups: c.includeSubgroups, projects: c.projectPaths.join('\n') };
}

function inputOf(f: Form): GitlabCiInput {
  return {
    baseUrl: f.install === 'saas' ? SAAS_URL : f.baseUrl.trim(),
    token: f.token.trim() || null,
    groupPath: f.groupPath.trim(),
    includeSubgroups: f.includeSubgroups,
    projectPaths: f.projects
      .split(/[\s,]+/)
      .map((p) => p.trim())
      .filter(Boolean),
  };
}

/** Where a token with read_api is created on the instance. */
function tokenUrl(f: Form): string | null {
  const base = f.install === 'saas' ? SAAS_URL : f.baseUrl.trim().replace(/\/+$/, '');
  if (!/^https?:\/\/[^\s]+$/.test(base)) return null;
  return `${base}/-/user_settings/personal_access_tokens?name=ObserveCraft&scopes=read_api`;
}

export function GitlabCiSetup() {
  const status = useGitlabCi();
  const { isAdmin } = useAccess();
  const { test, save, remove, sync } = useGitlabCiMutations();
  const [form, setForm] = useState<Form>(() => formOf(undefined));
  const [loadedAt, setLoadedAt] = useState<number | null>(null);
  const saved = status.data?.connection ?? null;

  // Fill the form once from the saved connection (and again after it changes).
  useEffect(() => {
    const at = status.data?.connection?.updatedAt ?? 0;
    if (status.data && at !== loadedAt) {
      setForm(formOf(status.data));
      setLoadedAt(at);
    }
  }, [status.data, loadedAt]);

  if (status.isPending) return <Loading />;
  if (status.isError) return <ErrorState error={status.error} onRetry={() => status.refetch()} />;

  const set = (patch: Partial<Form>) => {
    setForm((f) => ({ ...f, ...patch }));
    test.reset();
  };
  const input = inputOf(form);
  const urlOk = form.install === 'saas' || /^https?:\/\/[^\s/]+/.test(form.baseUrl.trim());
  const targetOk = !!input.groupPath || input.projectPaths.length > 0;
  const tokenOk = !!input.token || (!!saved && saved.baseUrl === input.baseUrl);
  const ready = urlOk && targetOk && tokenOk;
  const link = tokenUrl(form);
  const s = status.data;
  const errors = [s.discoveryError && { what: 'Project list', ...s.discoveryError }, ...s.projects.filter((p) => p.error).map((p) => ({ what: p.path, ...p.error! }))].filter(Boolean) as {
    what: string;
    message: string;
    at: number;
  }[];

  return (
    <>
      <Step n={2} title="Choose the installation type" open onToggle={() => {}} done={urlOk}>
        <div className="aws-methods ci-installs">
          {INSTALLS.map((m) => (
            <button key={m.id} type="button" className={`aws-method${form.install === m.id ? ' selected' : ''}`} onClick={() => set({ install: m.id })} disabled={!isAdmin}>
              <strong>
                {form.install === m.id && <Check size={15} />} {m.title}
              </strong>
              <span>{m.text}</span>
            </button>
          ))}
        </div>
        {form.install === 'self-managed' && (
          <label className="rt-field">
            <span>GitLab URL</span>
            <input value={form.baseUrl} onChange={(e) => set({ baseUrl: e.target.value })} placeholder="https://gitlab.example.com" disabled={!isAdmin} />
            <span className="muted">ObserveCraft calls the instance&apos;s REST API (/api/v4) from its servers; GitLab needs no access to ObserveCraft.</span>
          </label>
        )}
      </Step>

      <Step n={3} title="Configure the connection" open onToggle={() => {}} done={tokenOk && targetOk}>
        <label className="rt-field">
          <span>Access token</span>
          <input
            type="password"
            value={form.token}
            onChange={(e) => set({ token: e.target.value })}
            autoComplete="new-password"
            placeholder={saved && saved.baseUrl === input.baseUrl ? 'Saved (leave empty to keep it)' : 'glpat-…'}
            disabled={!isAdmin}
          />
          <span className="muted">
            A personal, group or project access token with the <code>read_api</code> scope
            {link ? (
              <>
                {' '}
                (
                <a href={link} target="_blank" rel="noreferrer noopener">
                  create one
                </a>
                )
              </>
            ) : null}
            . It is stored encrypted and never shown again.
          </span>
        </label>
        <label className="rt-field">
          <span>Group</span>
          <input value={form.groupPath} onChange={(e) => set({ groupPath: e.target.value })} placeholder="example-org" disabled={!isAdmin} />
        </label>
        <label className="ci-check">
          <input type="checkbox" checked={form.includeSubgroups} onChange={(e) => set({ includeSubgroups: e.target.checked })} disabled={!isAdmin} /> Include subgroups
        </label>
        <label className="rt-field">
          <span>Projects</span>
          <textarea rows={3} value={form.projects} onChange={(e) => set({ projects: e.target.value })} placeholder={'example-org/storefront-api\nexample-org/billing'} disabled={!isAdmin} />
          <span className="muted">Project paths to follow besides the group&apos;s, one per line. Follow a group, projects, or both.</span>
        </label>
      </Step>

      <Step n={4} title="Test and save" open onToggle={() => {}} done={!!saved}>
        {!isAdmin && <p className="muted">Only organization admins can change the GitLab connection.</p>}
        {test.data && (
          <div className={test.data.ok ? 'rt-note' : 'rt-warning'}>
            {test.data.ok ? <CircleCheck size={16} /> : <AlertTriangle size={16} />}
            <div>
              <div>{test.data.message}</div>
              {test.data.scopes.length > 0 && <div className="muted">Token scopes: {test.data.scopes.join(', ')}</div>}
              {test.data.sample.length > 0 && (
                <div className="muted">
                  {test.data.sample.join(', ')}
                  {test.data.projects > test.data.sample.length ? `, and ${test.data.projects - test.data.sample.length} more` : ''}
                </div>
              )}
            </div>
          </div>
        )}
        {[test.error, save.error, remove.error].filter(Boolean).map((e, i) => (
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
              onClick={() => window.confirm('Disconnect GitLab? Pipelines already imported stay.') && remove.mutate()}
            >
              Disconnect
            </button>
          )}
        </div>
      </Step>

      <Step n={5} title="Wait for the first pipelines" open onToggle={() => {}} done={s.pipelinesImported > 0}>
        {!saved ? (
          <p className="muted">Once the connection is saved, ObserveCraft reads the last 7 days of finished pipelines, then new ones every minute.</p>
        ) : (
          <>
            <dl className="ci-sync">
              <dt>GitLab</dt>
              <dd>{saved.baseUrl}</dd>
              <dt>Projects followed</dt>
              <dd>{s.discoveredAt ? s.projects.length.toLocaleString() : 'Listing…'}</dd>
              <dt>Pipelines imported</dt>
              <dd>
                {s.pipelinesImported.toLocaleString()} <span className="muted">({s.jobsImported.toLocaleString()} jobs)</span>
              </dd>
              <dt>Last sync</dt>
              <dd>{s.lastSyncAt ? fmtAgo(s.lastSyncAt) : 'Waiting for the first sync…'}</dd>
            </dl>
            {errors.length > 0 && (
              <div className="rt-warning">
                <AlertTriangle size={16} />
                <div>
                  {errors.slice(0, 5).map((e) => (
                    <div key={e.what}>
                      <b>{e.what}</b>: {e.message} <span className="muted">({fmtAgo(e.at)})</span>
                    </div>
                  ))}
                  {errors.length > 5 && <div className="muted">and {errors.length - 5} more</div>}
                </div>
              </div>
            )}
            <div className="rt-create-actions">
              <button type="button" className="btn" disabled={!isAdmin || sync.isPending} onClick={() => sync.mutate()}>
                <RefreshCw size={14} /> {sync.isPending ? 'Syncing…' : 'Sync now'}
              </button>
              <Link className="btn btn-primary" to="/ci/explorer">
                View pipelines
              </Link>
            </div>
          </>
        )}
      </Step>
    </>
  );
}
