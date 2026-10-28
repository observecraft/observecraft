import { useEffect, useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { AtSign, Check, ChevronDown, ChevronRight, Hash, Info, Send, Webhook } from 'lucide-react';
import type { OnCallChannelOption, OnCallProfile } from '../../api/onCall/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { errorText } from './common';
import { useOnCallProfile, useTestOnCallProfile, useUpdateOnCallProfile } from './queries';

function ChannelIcon({ kind }: { kind: string }) {
  return kind === 'email' ? <AtSign size={14} /> : kind === 'slack' ? <Hash size={14} /> : <Webhook size={14} />;
}

const channelLabel = (c: OnCallChannelOption, email: string) => (c.kind === 'email' ? `Email (${email})` : `@${c.handle}`);

/** The three setup steps of a member's On-Call profile: email, a chat channel, preferences per urgency. */
export function ProfileSetup({ initialStep = 1 }: { initialStep?: number }) {
  const q = useOnCallProfile();
  if (q.isPending) return <Loading label="Loading your On-Call profile…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <Steps profile={q.data} initialStep={initialStep} />;
}

function Steps({ profile, initialStep }: { profile: OnCallProfile; initialStep: number }) {
  const [open, setOpen] = useState<number | null>(initialStep);
  const [high, setHigh] = useState(profile.highUrgency);
  const [low, setLow] = useState(profile.lowUrgency);
  const save = useUpdateOnCallProfile();
  const test = useTestOnCallProfile();
  useEffect(() => {
    setHigh(profile.highUrgency);
    setLow(profile.lowUrgency);
  }, [profile]);
  const chat = profile.channels.filter((c) => c.kind !== 'email');
  const chosenChat = high.find((h) => h !== 'email') ?? '';
  const toggle = (list: string[], set: (v: string[]) => void, h: string) => set(list.includes(h) ? list.filter((x) => x !== h) : [...list, h]);
  const persist = (next: { highUrgency: string[]; lowUrgency: string[] }) => save.mutate(next);

  const step = (n: number, title: string, done: boolean, body: ReactNode) => (
    <div className={`onc-step${open === n ? ' open' : ''}`}>
      <button className="onc-step-head" onClick={() => setOpen(open === n ? null : n)}>
        <span className={`onc-step-num${done ? ' done' : ''}`}>{done ? <Check size={13} /> : n}</span>
        <span className="onc-step-title">{title}</span>
        {open === n ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
      </button>
      {open === n && <div className="onc-step-body">{body}</div>}
    </div>
  );

  return (
    <div className="onc-steps">
      {step(
        1,
        'Confirm the email address pages are sent to',
        profile.configured,
        <>
          <p>
            Pages reach you at <b>{profile.email}</b>, the address of your ObserveCraft account. ObserveCraft notifies by email, Slack and webhooks; it doesn't call or text phones.
          </p>
          {!profile.emailEnabled && (
            <p className="onc-note">
              <Info size={14} /> Email isn't configured on this install (<code>OC_SMTP_URL</code>): add a chat channel below so pages still reach you.
            </p>
          )}
          <div className="onc-step-actions">
            <button className="btn" disabled={test.isPending} onClick={() => test.mutate()}>
              <Send size={14} /> Send Test Notification
            </button>
            <button className="btn btn-primary" onClick={() => (persist({ highUrgency: high, lowUrgency: low }), setOpen(2))}>
              Confirm
            </button>
          </div>
          {test.data && (
            <ul className="onc-test-results">
              {test.data.map((r) => (
                <li key={r.channel} className={r.status === 'sent' ? 'ok' : 'bad'}>
                  {r.channel === 'email' ? 'Email' : `@${r.channel}`}: {r.status === 'sent' ? 'sent' : r.error}
                </li>
              ))}
            </ul>
          )}
        </>,
      )}
      {step(
        2,
        'Add a chat channel (optional)',
        !!chosenChat,
        <>
          <p>Also post high-urgency pages to a Slack or webhook channel of your organization, for example the one your team watches.</p>
          {chat.length === 0 ? (
            <p className="muted">
              No Slack or webhook channels yet. An admin adds them under <Link to="/monitors/settings">Monitors → Settings → Notification Channels</Link>.
            </p>
          ) : (
            <div className="onc-channel-list">
              {chat.map((c) => (
                <label key={c.handle} className="onc-channel">
                  <input type="checkbox" checked={high.includes(c.handle)} onChange={() => toggle(high, setHigh, c.handle)} />
                  <ChannelIcon kind={c.kind} />@{c.handle}
                </label>
              ))}
            </div>
          )}
          <div className="onc-step-actions">
            <button className="btn btn-primary" disabled={save.isPending || !high.length} onClick={() => (persist({ highUrgency: high, lowUrgency: low }), setOpen(3))}>
              Save and Continue
            </button>
          </div>
        </>,
      )}
      {step(
        3,
        'Set your notification preferences',
        profile.configured && (profile.highUrgency.length > 1 || profile.lowUrgency.join() !== profile.highUrgency.join()),
        <>
          <p>Choose how each urgency reaches you. Monitors with priority P3–P5 page with low urgency.</p>
          <table className="onc-pref-table">
            <thead>
              <tr>
                <th>CHANNEL</th>
                <th>HIGH URGENCY</th>
                <th>LOW URGENCY</th>
              </tr>
            </thead>
            <tbody>
              {profile.channels.map((c) => (
                <tr key={c.handle}>
                  <td>
                    <span className="onc-channel-name">
                      <ChannelIcon kind={c.kind} />
                      {channelLabel(c, profile.email)}
                    </span>
                  </td>
                  <td>
                    <input type="checkbox" aria-label={`High urgency by ${c.handle}`} checked={high.includes(c.handle)} onChange={() => toggle(high, setHigh, c.handle)} />
                  </td>
                  <td>
                    <input type="checkbox" aria-label={`Low urgency by ${c.handle}`} checked={low.includes(c.handle)} onChange={() => toggle(low, setLow, c.handle)} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="onc-step-actions">
            <button className="btn btn-primary" disabled={save.isPending || !high.length || !low.length} onClick={() => persist({ highUrgency: high, lowUrgency: low })}>
              Save Preferences
            </button>
            {save.isSuccess && !save.isPending && <span className="form-ok">Saved</span>}
          </div>
        </>,
      )}
      {save.error && <p className="form-error">{errorText(save.error)}</p>}
    </div>
  );
}
