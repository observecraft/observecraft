import { useEffect, useState } from 'react';
import { CheckCircle2, CircleAlert, CircleDashed, Trash2 } from 'lucide-react';
import type { PortalConnectionTest, PortalConnector, PortalIdentity, PortalProvider } from '../../api/portal/client';
import { age } from '../../api/portal/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { useProfile } from '../settings/queries';
import { useConnectors, useDeleteConnector, useIdentities, useSaveConnector, useSaveIdentities, useTestConnector } from './homeQueries';

const INFO: Record<PortalProvider, { label: string; text: string; baseUrl: string; basePlaceholder: string; token: string; identity: string }> = {
  github: {
    label: 'GitHub',
    text: 'Pull requests on the portal home. Use a token that can read the repositories of your organization; for GitHub Enterprise Server, set its API URL (https://github.example.com/api/v3).',
    baseUrl: 'API URL',
    basePlaceholder: 'https://api.github.com',
    token: 'Personal access token',
    identity: 'GitHub username',
  },
  gitlab: {
    label: 'GitLab',
    text: 'Merge requests on the portal home. Use a token with the read_api scope; for self-managed GitLab, set its URL.',
    baseUrl: 'GitLab URL',
    basePlaceholder: 'https://gitlab.com',
    token: 'Access token',
    identity: 'GitLab username',
  },
  jira: {
    label: 'Jira',
    text: 'Work items assigned to each member, found by their email. Use an API token of an account that can browse your projects.',
    baseUrl: 'Site URL',
    basePlaceholder: 'https://your-site.atlassian.net',
    token: 'API token',
    identity: 'Jira account email',
  },
  linear: {
    label: 'Linear',
    text: 'Issues assigned to each member, found by their email. Use a personal API key of a workspace member.',
    baseUrl: 'API URL',
    basePlaceholder: 'https://api.linear.app',
    token: 'API key',
    identity: 'Linear email',
  },
};

function ConnectionStatus({ c, test }: { c: PortalConnector; test: PortalConnectionTest | null }) {
  if (!c.configured) {
    return (
      <span className="ps-conn-status">
        <CircleDashed size={14} /> Not configured
      </span>
    );
  }
  const ok = test ? test.ok : c.lastCheckOk;
  const checkedAt = test ? test.checkedAt : c.lastCheckedAt;
  const error = test ? (test.ok ? '' : test.message) : c.lastCheckError;
  return (
    <span className={`ps-conn-status ${ok === false ? 'bad' : 'good'}`}>
      {ok === false ? <CircleAlert size={14} /> : <CheckCircle2 size={14} />}
      Configured{checkedAt ? ` · last checked ${age(checkedAt)}` : ' · not checked yet'}
      {test?.ok && <span className="sc-muted"> · {test.message}</span>}
      {error && <span className="ps-conn-error">{error}</span>}
    </span>
  );
}

function ConnectorCard({ c, isAdmin }: { c: PortalConnector; isAdmin: boolean }) {
  const info = INFO[c.provider];
  const [baseUrl, setBaseUrl] = useState(c.configured ? c.baseUrl : '');
  const [account, setAccount] = useState(c.account);
  const [token, setToken] = useState('');
  const [test, setTest] = useState<PortalConnectionTest | null>(null);
  const save = useSaveConnector();
  const remove = useDeleteConnector();
  const check = useTestConnector();
  useEffect(() => setTest(null), [c.updatedAt]);
  const error = save.error ?? remove.error ?? check.error;
  const editable = isAdmin && c.provider !== 'linear';
  return (
    <div className="ps-conn">
      <div className="ps-conn-head">
        <span className="ps-conn-logo">
          <IntegrationLogo logo={c.provider} name={info.label} size={24} />
        </span>
        <div className="ps-conn-title">
          <strong>{info.label}</strong>
          <ConnectionStatus c={c} test={test} />
        </div>
        {isAdmin && c.configured && (
          <button
            className="sc-icon-btn"
            aria-label={`Remove the ${info.label} connection`}
            title="Remove"
            onClick={() => window.confirm(`Remove the ${info.label} connection and its token?`) && remove.mutate(c.provider)}
          >
            <Trash2 size={14} />
          </button>
        )}
      </div>
      <p className="ps-conn-text">{info.text}</p>
      {isAdmin ? (
        <form
          className="ps-conn-form"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate({ provider: c.provider, input: { baseUrl, account, token: token.trim() || null } }, { onSuccess: () => setToken('') });
          }}
        >
          <label className="ph-field">
            <span>{info.baseUrl}</span>
            <input className="sc-input" value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder={info.basePlaceholder} disabled={!editable} />
          </label>
          {c.provider === 'jira' && (
            <label className="ph-field">
              <span>Account email</span>
              <input className="sc-input" value={account} onChange={(e) => setAccount(e.target.value)} placeholder="portal-bot@example.com" />
            </label>
          )}
          <label className="ph-field">
            <span>{info.token}</span>
            <input
              className="sc-input mono"
              type="password"
              autoComplete="new-password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder={c.configured ? 'Stored · enter a new one to replace it' : 'Paste the token'}
            />
          </label>
          <div className="ps-conn-actions">
            <button type="submit" className="btn btn-primary btn-sm" disabled={save.isPending || (!c.configured && !token.trim())}>
              {c.configured ? 'Save' : 'Connect'}
            </button>
            <button type="button" className="btn btn-sm" disabled={!c.configured || check.isPending} onClick={() => check.mutate(c.provider, { onSuccess: setTest })}>
              {check.isPending ? 'Testing…' : 'Test connection'}
            </button>
          </div>
          {error && <div className="ph-form-error">{error.message}</div>}
        </form>
      ) : (
        <p className="sc-muted ps-conn-text">Only organization admins can change connections.</p>
      )}
    </div>
  );
}

function IdentityForm({ identities, email }: { identities: PortalIdentity[]; email: string }) {
  const [values, setValues] = useState<Record<string, string>>(() => Object.fromEntries(identities.map((i) => [i.provider, i.isDefault ? '' : i.handle])));
  const save = useSaveIdentities();
  return (
    <form
      className="ps-identities"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate({ identities: identities.map((i) => ({ provider: i.provider, handle: values[i.provider] ?? '' })) });
      }}
    >
      {identities.map((i) => (
        <label key={i.provider} className="ps-identity">
          <span className="ps-identity-label">
            <IntegrationLogo logo={i.provider} name={INFO[i.provider].label} size={16} />
            {INFO[i.provider].identity}
          </span>
          <input
            className="sc-input"
            value={values[i.provider] ?? ''}
            onChange={(e) => setValues({ ...values, [i.provider]: e.target.value })}
            placeholder={i.provider === 'jira' || i.provider === 'linear' ? email || 'you@example.com' : 'username'}
          />
        </label>
      ))}
      <div className="ps-conn-actions">
        <button type="submit" className="btn btn-primary btn-sm" disabled={save.isPending}>
          Save identities
        </button>
        {save.isSuccess && <span className="sc-muted">Saved</span>}
      </div>
      {save.error && <div className="ph-form-error">{save.error.message}</div>}
    </form>
  );
}

/** Settings → Configurations: the org's source connectors and the member's identity on each provider. */
export function SettingsConfigurations() {
  const me = useProfile();
  const connectors = useConnectors();
  const identities = useIdentities();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  return (
    <div className="ps-page">
      <h1 className="ps-title">Configurations</h1>
      <p className="sc-muted ps-lead">
        Connect the code hosts and trackers your teams use. Tokens are encrypted at rest and never shown again; the server calls the providers on behalf of each member.
      </p>
      {connectors.isLoading ? (
        <Loading />
      ) : connectors.error ? (
        <ErrorState error={connectors.error} onRetry={() => connectors.refetch()} />
      ) : (
        <div className="ps-conn-grid">
          {connectors.data!.map((c) => (
            <ConnectorCard key={`${c.provider}-${c.updatedAt ?? 'new'}`} c={c} isAdmin={isAdmin} />
          ))}
        </div>
      )}
      <h2 className="ps-h2" id="identities">
        Your identities
      </h2>
      <p className="sc-muted ps-lead">Who you are on each provider, so the home page shows your pull requests and tickets. Jira and Linear use your ObserveCraft email unless you set another.</p>
      <div className="ph-card">
        {identities.isLoading ? (
          <Loading />
        ) : identities.error ? (
          <ErrorState error={identities.error} onRetry={() => identities.refetch()} />
        ) : (
          <IdentityForm key={identities.dataUpdatedAt} identities={identities.data!} email={me.data?.email ?? ''} />
        )}
      </div>
    </div>
  );
}
