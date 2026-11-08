// Integrations → Slack: connected workspaces and the channels monitors mention as
// @slack-<account>-<channel> (the rows Terraform's datadog_integration_slack_channel manages too),
// with what each notification shows. Shares the account layout of the cloud pages (aws.css).
import { useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { Check, ChevronRight, Copy, Info, Lock, Plus, Send, Trash2, TriangleAlert, X } from 'lucide-react';
import { DEFAULT_DISPLAY, type SlackAccount, type SlackChannel, type SlackDisplay, type SlackOverview } from '../../../api/slack/client';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { IntegrationLogo } from '../IntegrationLogo';
import { useSlackConversations, useSlackMutations, useSlackOverview } from './queries';
import '../../settings/settings.css';
import '../integrations.css';
import '../aws/aws.css';
import './slack.css';
import { HighlightedCode } from '../../../components/code/highlight';

const DESCRIPTION =
  'Slack is a messaging platform for teams. Connect a workspace to send monitor alerts, On-Call pages, workflow messages and reports to its channels, and to mute monitors from the alert.';

const DISPLAY_OPTIONS: { key: keyof SlackDisplay; label: string; hint: string }[] = [
  { key: 'message', label: 'Message', hint: "The monitor's message" },
  { key: 'notified', label: 'Notified', hint: 'The @-handles the message notified' },
  { key: 'tags', label: 'Tags', hint: 'The scope that alerted and the monitor tags' },
  { key: 'snapshot', label: 'Snapshot', hint: 'A graph snapshot (stored for Terraform; not drawn yet)' },
  { key: 'mute_buttons', label: 'Mute buttons', hint: 'Mute 1h / 4h / 1d from the alert (needs interactivity)' },
];

export function SlackPage() {
  const [params, setParams] = useSearchParams();
  const overview = useSlackOverview();
  const tab = params.get('tab') === 'setup' ? 'setup' : 'configuration';
  const setParam = (key: string, value: string | null) =>
    setParams((p) => {
      const next = new URLSearchParams(p);
      if (value) next.set(key, value);
      else next.delete(key);
      return next;
    });
  const connected = params.get('connected');
  const error = params.get('error');
  const installed = (overview.data?.accounts.length ?? 0) > 0;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>Slack</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo="slack" name="Slack" size={72} />
        <div>
          <h1>Slack</h1>
          <p>{DESCRIPTION}</p>
        </div>
        {installed && (
          <span className="btn aws-installed">
            <Check size={15} /> Installed
          </span>
        )}
      </header>
      {(connected || error) && (
        <div className={connected ? 'aws-note slack-banner' : 'aws-issue slack-banner'}>
          {connected ? <Check size={16} /> : <TriangleAlert size={16} />}
          <span>{connected ? `Connected the workspace as ${connected}. Add the channels monitors should post to.` : error}</span>
          <button className="icon-plain" title="Dismiss" onClick={() => setParams({}, { replace: true })}>
            <X size={14} />
          </button>
        </div>
      )}
      <nav className="im-tabs aws-tabs">
        <button className={tab === 'configuration' ? 'active' : ''} onClick={() => setParam('tab', null)}>
          Configuration
        </button>
        <button className={tab === 'setup' ? 'active' : ''} onClick={() => setParam('tab', 'setup')}>
          Set Up a Slack App
        </button>
      </nav>
      {overview.isPending ? (
        <Loading label="Loading Slack workspaces…" />
      ) : overview.isError ? (
        <ErrorState error={overview.error} onRetry={() => overview.refetch()} />
      ) : tab === 'setup' ? (
        <AppSetup overview={overview.data} />
      ) : overview.data.accounts.length === 0 || params.get('add') ? (
        <Connect overview={overview.data} onCancel={overview.data.accounts.length ? () => setParam('add', null) : undefined} onConnected={(id) => setParams({ account: id }, { replace: true })} />
      ) : (
        <Accounts overview={overview.data} selected={params.get('account')} onSelect={(id) => setParam('account', id)} onAdd={() => setParam('add', '1')} />
      )}
    </div>
  );
}

// ------------------------------------------------------------------------------------------- connect

function Connect({ overview, onCancel, onConnected }: { overview: SlackOverview; onCancel?: () => void; onConnected: (id: string) => void }) {
  const { connect, oauth } = useSlackMutations();
  const [name, setName] = useState(overview.accounts.length ? '' : 'main');
  const [token, setToken] = useState('');
  const [secret, setSecret] = useState('');
  const addToSlack = () => oauth.mutate(name, { onSuccess: ({ url }) => window.location.assign(url) });
  return (
    <div className="aws-form slack-connect">
      <h2>Connect a Slack workspace</h2>
      <label className="rt-field">
        <span>Account name</span>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="main" />
        <small className="muted">
          Monitors mention its channels as <code>@slack-{name.trim().toLowerCase() || '<account>'}-&lt;channel&gt;</code> (or <code>@slack-&lt;channel&gt;</code>). It&apos;s also the{' '}
          <code>account_name</code> of Terraform&apos;s <code>datadog_integration_slack_channel</code>.
        </small>
      </label>
      {overview.oauthAvailable && (
        <>
          <button className="btn btn-primary slack-add" disabled={!name.trim() || oauth.isPending} onClick={addToSlack}>
            <IntegrationLogo logo="slack" name="Slack" size={16} /> {oauth.isPending ? 'Opening Slack…' : 'Add to Slack'}
          </button>
          {oauth.error && <p className="form-error">{(oauth.error as Error).message}</p>}
          <p className="muted slack-or">or connect the workspace&apos;s own Slack app</p>
        </>
      )}
      {!overview.oauthAvailable && (
        <div className="aws-note">
          <Info size={16} />
          <span>
            This install has no shared Slack app (<code>OC_SLACK_CLIENT_ID</code>, <code>OC_SLACK_CLIENT_SECRET</code>). Create one in your workspace from the manifest under{' '}
            <Link to="?tab=setup">Set Up a Slack App</Link> and paste its tokens here.
          </span>
        </div>
      )}
      <label className="rt-field">
        <span>Bot User OAuth Token</span>
        <input type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" placeholder="xoxb-…" />
      </label>
      <label className="rt-field">
        <span>Signing secret (optional)</span>
        <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="off" placeholder="For the mute buttons" />
      </label>
      {connect.error && <p className="form-error">{(connect.error as Error).message}</p>}
      <div className="aws-save">
        {onCancel && (
          <button className="btn" onClick={onCancel}>
            Cancel
          </button>
        )}
        <button
          className="btn btn-primary"
          disabled={!name.trim() || !token.trim() || connect.isPending}
          onClick={() => connect.mutate({ name, botToken: token.trim(), signingSecret: secret.trim() || null }, { onSuccess: (a) => onConnected(a.id) })}
        >
          {connect.isPending ? 'Checking the token…' : 'Connect'}
        </button>
      </div>
    </div>
  );
}

function CopyButton({ text, label = 'Copy' }: { text: string; label?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      className="btn btn-sm"
      onClick={() =>
        navigator.clipboard?.writeText(text).then(() => {
          setDone(true);
          setTimeout(() => setDone(false), 1500);
        })
      }
    >
      {done ? <Check size={13} /> : <Copy size={13} />} {done ? 'Copied' : label}
    </button>
  );
}

function AppSetup({ overview }: { overview: SlackOverview }) {
  return (
    <div className="aws-form slack-setup">
      <h2>Create a Slack app for this install</h2>
      <ol className="slack-steps">
        <li>
          In Slack&apos;s app settings, choose <strong>Create New App → From a manifest</strong>, pick the workspace and paste this manifest:
          <div className="slack-manifest">
            <pre className="mono">
              <HighlightedCode code={overview.appManifest} language="yaml" />
            </pre>
            <CopyButton text={overview.appManifest} label="Copy manifest" />
          </div>
        </li>
        <li>
          <strong>Install to Workspace</strong>, then copy the <strong>Bot User OAuth Token</strong> (OAuth &amp; Permissions) and the <strong>Signing Secret</strong> (Basic Information).
        </li>
        <li>
          Back on <Link to="/integrations/slack?add=1">Configuration</Link>, connect the workspace with them.
        </li>
        <li>
          Invite the bot to private channels it should post to (<code>/invite @ObserveCraft</code>); public channels need nothing.
        </li>
      </ol>
      <h4 className="aws-subhead">One app for every organization of this install</h4>
      <p className="muted">
        Set <code>OC_SLACK_CLIENT_ID</code>, <code>OC_SLACK_CLIENT_SECRET</code> and <code>OC_SLACK_SIGNING_SECRET</code> from the app above (with public distribution enabled) and add this redirect
        URL under OAuth &amp; Permissions: organizations then connect with <strong>Add to Slack</strong>.
      </p>
      <dl className="aws-facts">
        <dt>Redirect URL</dt>
        <dd className="mono">
          {overview.redirectUrl} <CopyButton text={overview.redirectUrl} />
        </dd>
        <dt>Interactivity URL</dt>
        <dd className="mono">
          {overview.interactivityUrl} <CopyButton text={overview.interactivityUrl} />
        </dd>
        <dt>Bot scopes</dt>
        <dd className="mono">{overview.scopes.join(', ')}</dd>
      </dl>
    </div>
  );
}

// ------------------------------------------------------------------------------------------- accounts

function Accounts({ overview, selected, onSelect, onAdd }: { overview: SlackOverview; selected: string | null; onSelect: (id: string | null) => void; onAdd: () => void }) {
  const current = overview.accounts.find((a) => a.id === selected) ?? overview.accounts[0];
  return (
    <>
      <div className="aws-toolbar">
        <span className="aws-search-spacer" />
        <button className="btn btn-primary" onClick={onAdd}>
          <Plus size={15} /> Add Workspace
        </button>
      </div>
      <div className="aws-split">
        <aside className="aws-account-list">
          <h3>Showing {overview.accounts.length} Workspaces</h3>
          {overview.accounts.map((a) => (
            <button key={a.id} className={a.id === current.id ? 'selected' : ''} onClick={() => onSelect(a.id)}>
              <strong>{a.teamName || a.name}</strong>
              <span className="aws-tags">
                <span className="aws-tag">account:{a.name}</span>
                <span className="aws-tag">{a.channels.length} channels</span>
              </span>
            </button>
          ))}
        </aside>
        <Workspace key={current.id} account={current} onDisconnected={() => onSelect(null)} />
      </div>
    </>
  );
}

function Workspace({ account, onDisconnected }: { account: SlackAccount; onDisconnected: () => void }) {
  const { disconnect } = useSlackMutations();
  const [confirm, setConfirm] = useState(false);
  return (
    <section className="aws-detail">
      <header>
        <h2>{account.teamName || account.name}</h2>
      </header>
      <div className="aws-detail-body">
        <div className="aws-form">
          <dl className="aws-facts">
            <dt>Account name</dt>
            <dd className="mono">{account.name}</dd>
            <dt>Workspace</dt>
            <dd className="mono">{account.teamId}</dd>
            <dt>Connected</dt>
            <dd>
              {timeAgo(account.createdAt)} · {account.installedVia === 'oauth' ? 'Add to Slack' : "the workspace's own app"}
            </dd>
            <dt>Mute buttons</dt>
            <dd>{account.interactive ? 'Available' : 'Unavailable: no signing secret (reconnect with the app’s signing secret)'}</dd>
          </dl>
          {account.missingScopes.length > 0 && (
            <div className="aws-issue">
              <strong>The app wasn&apos;t granted every permission.</strong>
              <p>
                Missing: <code>{account.missingScopes.join(', ')}</code>. Reinstall it (Add Workspace with the same account name) so channels can be listed and mute buttons identify who pressed them.
              </p>
            </div>
          )}
          <Channels account={account} />
          <div className="aws-danger">
            {confirm ? (
              <>
                <span>Disconnect this workspace? Its {account.channels.length} channels are removed and monitors mentioning them stop notifying them. The app stays installed in Slack.</span>
                <button className="btn" onClick={() => setConfirm(false)}>
                  Cancel
                </button>
                <button className="btn btn-danger" disabled={disconnect.isPending} onClick={() => disconnect.mutate(account.id, { onSuccess: onDisconnected })}>
                  {disconnect.isPending ? 'Disconnecting…' : 'Disconnect'}
                </button>
              </>
            ) : (
              <button className="btn" onClick={() => setConfirm(true)}>
                <Trash2 size={14} /> Disconnect workspace
              </button>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}

function Channels({ account }: { account: SlackAccount }) {
  const m = useSlackMutations();
  const conversations = useSlackConversations(account.id);
  const [name, setName] = useState('');
  const [tested, setTested] = useState<Record<string, { ok: boolean; text: string }>>({});
  const configured = new Set(account.channels.map((c) => c.name.replace(/^#/, '').toLowerCase()));
  const choices = (conversations.data ?? []).filter((c) => !configured.has(c.name.toLowerCase()));
  const add = () => m.addChannel.mutate({ accountId: account.id, name: name.startsWith('#') ? name : `#${name}`, display: DEFAULT_DISPLAY }, { onSuccess: () => setName('') });
  const test = (c: SlackChannel) =>
    m.testChannel.mutate(c.id, {
      onSuccess: (r) => setTested((t) => ({ ...t, [c.id]: { ok: r.status === 'sent', text: r.status === 'sent' ? 'Sent' : (r.error ?? r.status) } })),
      onError: (e) => setTested((t) => ({ ...t, [c.id]: { ok: false, text: (e as Error).message } })),
    });
  const setDisplay = (c: SlackChannel, key: keyof SlackDisplay, value: boolean) => m.updateChannel.mutate({ id: c.id, display: { ...c.display, [key]: value } });
  return (
    <>
      <h4 className="aws-subhead">Channels</h4>
      <div className="slack-add-channel">
        <input list={`slack-conv-${account.id}`} value={name} onChange={(e) => setName(e.target.value.trim())} placeholder="#channel" onKeyDown={(e) => e.key === 'Enter' && name && add()} />
        <datalist id={`slack-conv-${account.id}`}>
          {choices.map((c) => (
            <option key={c.id} value={`#${c.name}`}>
              {c.isPrivate ? 'private' : 'public'}
              {c.isPrivate && !c.isMember ? ' · invite the bot' : ''}
            </option>
          ))}
        </datalist>
        <button className="btn btn-primary" disabled={!name || m.addChannel.isPending} onClick={add}>
          <Plus size={14} /> Add Channel
        </button>
        {conversations.isError && <small className="muted">Channels can&apos;t be listed ({(conversations.error as Error).message}); type the name.</small>}
      </div>
      {(m.addChannel.error || m.updateChannel.error || m.removeChannel.error) && (
        <p className="form-error">{((m.addChannel.error || m.updateChannel.error || m.removeChannel.error) as Error).message}</p>
      )}
      {account.channels.length === 0 ? (
        <Empty>No channels yet. Add the channels monitors should post to.</Empty>
      ) : (
        <table className="im-table slack-channels">
          <thead>
            <tr>
              <th>CHANNEL</th>
              <th>MENTION AS</th>
              {DISPLAY_OPTIONS.map((o) => (
                <th key={o.key} title={o.hint} className="slack-opt">
                  {o.label.toUpperCase()}
                </th>
              ))}
              <th />
            </tr>
          </thead>
          <tbody>
            {account.channels.map((c) => {
              const conv = conversations.data?.find((x) => x.name.toLowerCase() === c.name.replace(/^#/, '').toLowerCase());
              return (
                <tr key={c.id}>
                  <td>
                    <span className="slack-channel-name">
                      {conv?.isPrivate && <Lock size={12} />}
                      {c.name}
                    </span>
                    {conv?.isPrivate && !conv.isMember && <small className="slack-warn">Invite the bot to this private channel</small>}
                    {conversations.isSuccess && !conv && <small className="slack-warn">Not visible to the bot</small>}
                  </td>
                  <td>
                    <code>@{c.handle}</code>
                  </td>
                  {DISPLAY_OPTIONS.map((o) => (
                    <td key={o.key} className="slack-opt">
                      <button
                        type="button"
                        role="switch"
                        aria-label={`${o.label} for ${c.name}`}
                        title={o.key === 'mute_buttons' && !account.interactive ? 'Needs the app’s signing secret' : o.hint}
                        aria-checked={c.display[o.key]}
                        className={`switch${c.display[o.key] ? ' on' : ''}`}
                        onClick={() => setDisplay(c, o.key, !c.display[o.key])}
                      />
                    </td>
                  ))}
                  <td className="slack-actions">
                    <button className="btn btn-sm" disabled={m.testChannel.isPending && m.testChannel.variables === c.id} onClick={() => test(c)} title="Send a sample alert">
                      <Send size={13} /> Test
                    </button>
                    <button
                      className="icon-plain"
                      title="Remove"
                      onClick={() => window.confirm(`Remove ${c.name}? Monitors mentioning @${c.handle} stop notifying it.`) && m.removeChannel.mutate(c.id)}
                    >
                      <Trash2 size={14} />
                    </button>
                    {tested[c.id] && <small className={tested[c.id].ok ? 'slack-ok' : 'slack-warn'}>{tested[c.id].text}</small>}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </>
  );
}
