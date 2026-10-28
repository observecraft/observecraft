import { BrandLogo } from '../../components/brand/BrandLogo';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronRight, Gauge, Trash2 } from 'lucide-react';
import { createChannel, deleteChannel, listChannels, listDeliveries } from '../../api/notifications/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { useProfile } from '../settings/queries';
import '../settings/settings.css';
import './monitors.css';

const STATUS_LABEL: Record<string, string> = { pending: 'Retrying', sent: 'Sent', failed: 'Failed', unknown_recipient: 'Not delivered' };

/** Monitors → Settings: where @handles in monitor messages deliver, and what was delivered. */
export function MonitorSettingsPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  const channels = useQuery({ queryKey: ['notification-channels'], queryFn: listChannels });
  const deliveries = useQuery({ queryKey: ['notification-deliveries'], queryFn: listDeliveries, refetchInterval: 15_000 });
  const [kind, setKind] = useState<'webhook' | 'slack'>('webhook');
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const [secret, setSecret] = useState('');
  const refresh = () => qc.invalidateQueries({ queryKey: ['notification-channels'] });
  const create = useMutation({
    mutationFn: () => createChannel({ kind, name, url, secret: kind === 'webhook' ? secret : null }),
    onSuccess: () => {
      setName('');
      setUrl('');
      setSecret('');
      void refresh();
    },
  });
  const remove = useMutation({ mutationFn: deleteChannel, onSuccess: refresh });
  return (
    <div className="mon-editor">
      <header className="mon-crumbs">
        <Link to="/monitors" className="mon-crumb-root">
          <Gauge size={22} /> Monitors
        </Link>
        <ChevronRight size={18} className="muted" />
        <span>Settings</span>
      </header>
      <div className="settings-pad mon-settings">
        <h2 className="settings-subtitle">Notification channels</h2>
        <p className="muted">
          Mention a channel in a monitor's message (<code>@webhook-oncall</code>, <code>@slack-checkout-alerts</code>) to notify it on every alert, warning, recovery and no-data transition. Emails (
          <code>@name@example.com</code>) go to members of this organization through the install's SMTP server.
        </p>
        {isAdmin && (
          <form
            className="mon-channel-form"
            onSubmit={(e) => {
              e.preventDefault();
              create.mutate();
            }}
          >
            <select value={kind} onChange={(e) => setKind(e.target.value as 'webhook' | 'slack')}>
              <option value="webhook">Webhook</option>
              <option value="slack">Slack</option>
            </select>
            <span className="mon-channel-handle">
              @{kind}-
              <input value={name} onChange={(e) => setName(e.target.value.toLowerCase())} placeholder="oncall" required maxLength={60} />
            </span>
            <input
              className="mon-channel-url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder={kind === 'slack' ? 'https://hooks.slack.com/services/…' : 'https://example.com/hooks/observecraft'}
              required
            />
            {kind === 'webhook' && <input value={secret} onChange={(e) => setSecret(e.target.value)} placeholder="Signing secret (optional)" type="password" autoComplete="new-password" />}
            <button className="btn btn-primary" disabled={create.isPending}>
              Add Channel
            </button>
          </form>
        )}
        {(create.error || remove.error) && <p className="form-error">{((create.error ?? remove.error) as Error).message}</p>}
        {channels.isPending ? (
          <Loading />
        ) : channels.isError ? (
          <ErrorState error={channels.error} />
        ) : channels.data.length === 0 ? (
          <p className="muted">No channels yet.</p>
        ) : (
          <table className="settings-table">
            <thead>
              <tr>
                <th>HANDLE</th>
                <th>TYPE</th>
                <th>DESTINATION</th>
                <th>SIGNED</th>
                <th>ADDED</th>
                {isAdmin && <th />}
              </tr>
            </thead>
            <tbody>
              {channels.data.map((c) => (
                <tr key={c.id}>
                  <td className="mono">@{c.handle}</td>
                  <td>
                    <span className="mon-channel-kind">
                      <BrandLogo name={c.kind === 'slack' ? 'slack' : 'webhook'} size={16} />
                      {c.kind === 'slack' ? 'Slack' : 'Webhook'}
                    </span>
                  </td>
                  <td>{c.target}</td>
                  <td>{c.kind === 'webhook' ? (c.signed ? 'HMAC-SHA256' : 'No') : '—'}</td>
                  <td className="muted">{timeAgo(c.createdAt)}</td>
                  {isAdmin && (
                    <td className="num">
                      <button className="icon-plain" title="Delete" onClick={() => window.confirm(`Delete @${c.handle}? Monitors mentioning it stop notifying it.`) && remove.mutate(c.id)}>
                        <Trash2 size={15} />
                      </button>
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <p className="muted small">
          Webhooks receive a JSON body (event, monitor, group, transition, value, message). With a secret, <code>X-ObserveCraft-Signature: sha256=&lt;hex&gt;</code> is the HMAC-SHA256 of the raw body.
        </p>

        <h2 className="settings-subtitle">Recent deliveries</h2>
        {deliveries.data && deliveries.data.length === 0 && <p className="muted">No notifications sent yet.</p>}
        {deliveries.data && deliveries.data.length > 0 && (
          <table className="settings-table">
            <thead>
              <tr>
                <th>WHEN</th>
                <th>MONITOR</th>
                <th>TRANSITION</th>
                <th>RECIPIENT</th>
                <th>STATUS</th>
              </tr>
            </thead>
            <tbody>
              {deliveries.data.map((d) => (
                <tr key={d.id}>
                  <td className="muted">{timeAgo(d.createdAt)}</td>
                  <td>
                    <Link to={`/monitors/${d.monitorId}`}>{d.monitorName}</Link>
                    {d.groupKey && d.groupKey !== '*' && <span className="muted"> · {d.groupKey}</span>}
                  </td>
                  <td>{d.toStatus}</td>
                  <td className="mono">@{d.recipient}</td>
                  <td title={d.error ?? ''} className={d.status === 'sent' ? 'mon-delivery-ok' : d.status === 'pending' ? '' : 'mon-delivery-bad'}>
                    {STATUS_LABEL[d.status] ?? d.status}
                    {d.attempts > 1 && <span className="muted"> ({d.attempts} attempts)</span>}
                    {d.error && d.status !== 'sent' && <div className="muted small">{d.error}</div>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
