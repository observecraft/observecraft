import { useMemo, useState } from 'react';
import qrcode from 'qrcode-generator';
import { Check, Copy, KeyRound, Laptop, ShieldCheck, Trash2 } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  changePassword,
  createAppKey,
  listAppKeys,
  listSessions,
  mfaDisable,
  mfaEnable,
  mfaSetup,
  revokeAppKey,
  revokeSession,
  securityStatus,
} from '../../api/auth/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';

const keys = { security: ['auth', 'security'] as const, sessions: ['auth', 'sessions'] as const, appKeys: ['auth', 'app-keys'] as const };

function useAuthMutation<A, R>(fn: (a: A) => Promise<R>) {
  const qc = useQueryClient();
  return useMutation({ mutationFn: fn, onSuccess: () => qc.invalidateQueries({ queryKey: ['auth'] }) });
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : 'Something went wrong';
}

function CopyButton({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className="btn btn-sm"
      onClick={() => {
        void navigator.clipboard?.writeText(text);
        setDone(true);
        setTimeout(() => setDone(false), 1500);
      }}
    >
      {done ? <Check size={14} /> : <Copy size={14} />} {done ? 'Copied' : 'Copy'}
    </button>
  );
}

/** Password, authenticator-app MFA and active sessions. */
export function SecuritySection() {
  const status = useQuery({ queryKey: keys.security, queryFn: securityStatus });
  if (status.isPending) return <Loading />;
  if (status.isError) return <ErrorState error={status.error} onRetry={() => status.refetch()} />;
  return (
    <div className="settings-pad">
      <h1 className="settings-title">Security</h1>
      <div className="profile-cards flush">
        <PasswordCard hasPassword={status.data.hasPassword} />
        <MfaCard enabled={status.data.mfaEnabled} />
        <SessionsCard />
      </div>
    </div>
  );
}

function PasswordCard({ hasPassword }: { hasPassword: boolean }) {
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const [confirm, setConfirm] = useState('');
  const [ok, setOk] = useState(false);
  const save = useAuthMutation(() => changePassword(next, hasPassword ? current : undefined));
  const mismatch = confirm.length > 0 && next !== confirm;
  return (
    <section className="settings-card">
      <h2>Password</h2>
      <p className="muted">{hasPassword ? 'Changing your password signs you out of every other session.' : 'You sign in with single sign-on. You can also set a password for direct sign-in, if your organization allows it.'}</p>
      <form
        className="security-form"
        onSubmit={(e) => {
          e.preventDefault();
          setOk(false);
          save.mutate(undefined, {
            onSuccess: () => {
              setOk(true);
              setCurrent('');
              setNext('');
              setConfirm('');
            },
          });
        }}
      >
        {hasPassword && (
          <label className="form-label">
            Current password
            <input className="settings-input" type="password" autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} required />
          </label>
        )}
        <label className="form-label">
          New password
          <input className="settings-input" type="password" autoComplete="new-password" minLength={12} value={next} onChange={(e) => setNext(e.target.value)} required />
        </label>
        <label className="form-label">
          Confirm new password
          <input className="settings-input" type="password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} required />
        </label>
        <button className="btn btn-primary" disabled={save.isPending || mismatch || next.length < 12}>
          {hasPassword ? 'Change password' : 'Set password'}
        </button>
      </form>
      <p className="muted small-note">At least 12 characters. A passphrase of several words is both strong and memorable.</p>
      {mismatch && <p className="form-error">Passwords don't match</p>}
      {save.isError && <p className="form-error">{errorText(save.error)}</p>}
      {ok && <p className="form-ok">Password updated.</p>}
    </section>
  );
}

function Qr({ text }: { text: string }) {
  const svg = useMemo(() => {
    const q = qrcode(0, 'M');
    q.addData(text);
    q.make();
    return q.createSvgTag({ cellSize: 4, margin: 2, scalable: true });
  }, [text]);
  // createSvgTag output is generated locally from our own otpauth URI, not user HTML.
  return <div className="mfa-qr" dangerouslySetInnerHTML={{ __html: svg }} />;
}

function MfaCard({ enabled }: { enabled: boolean }) {
  const [setup, setSetup] = useState<{ secret: string; otpauthUrl: string } | null>(null);
  const [code, setCode] = useState('');
  const start = useAuthMutation(() => mfaSetup());
  const enable = useAuthMutation((c: string) => mfaEnable(c));
  const disable = useAuthMutation((c: string) => mfaDisable(c));
  const err = start.error ?? enable.error ?? disable.error;
  return (
    <section className="settings-card">
      <h2>
        Two-factor authentication <span className={`status-pill status-${enabled ? 'active' : 'deactivated'}`}>{enabled ? 'ON' : 'OFF'}</span>
      </h2>
      {enabled ? (
        <>
          <p className="muted">Password sign-ins ask for a code from your authenticator app. Enter a current code to turn this off.</p>
          <form className="security-form" onSubmit={(e) => (e.preventDefault(), disable.mutate(code.replace(/\s/g, ''), { onSuccess: () => setCode('') }))}>
            <input className="settings-input code-input" inputMode="numeric" placeholder="123456" value={code} onChange={(e) => setCode(e.target.value)} required />
            <button className="btn btn-danger" disabled={disable.isPending}>
              Turn off
            </button>
          </form>
        </>
      ) : !setup ? (
        <>
          <p className="muted">Add a second step to password sign-in with an authenticator app (Microsoft Authenticator, Google Authenticator, 1Password, …). Single sign-on uses your identity provider's MFA.</p>
          <button className="btn btn-primary" onClick={() => start.mutate(undefined, { onSuccess: setSetup })} disabled={start.isPending}>
            <ShieldCheck size={14} /> Set up authenticator app
          </button>
        </>
      ) : (
        <div className="mfa-setup">
          <Qr text={setup.otpauthUrl} />
          <div>
            <ol className="mfa-steps">
              <li>Scan the QR code with your authenticator app.</li>
              <li>
                Or enter this key manually: <code className="mfa-secret">{setup.secret.replace(/(.{4})/g, '$1 ').trim()}</code> <CopyButton text={setup.secret} />
              </li>
              <li>Enter the 6-digit code the app shows.</li>
            </ol>
            <form
              className="security-form"
              onSubmit={(e) => {
                e.preventDefault();
                enable.mutate(code.replace(/\s/g, ''), { onSuccess: () => (setSetup(null), setCode('')) });
              }}
            >
              <input className="settings-input code-input" inputMode="numeric" autoComplete="one-time-code" placeholder="123456" value={code} onChange={(e) => setCode(e.target.value)} required autoFocus />
              <button className="btn btn-primary" disabled={enable.isPending}>
                Verify and turn on
              </button>
              <button type="button" className="btn" onClick={() => setSetup(null)}>
                Cancel
              </button>
            </form>
          </div>
        </div>
      )}
      {err && <p className="form-error">{errorText(err)}</p>}
    </section>
  );
}

function browserLabel(ua: string | null | undefined): string {
  if (!ua) return 'Unknown client';
  const browser = /Edg\//.test(ua) ? 'Edge' : /Chrome\//.test(ua) ? 'Chrome' : /Firefox\//.test(ua) ? 'Firefox' : /Safari\//.test(ua) ? 'Safari' : /curl|python|Go-http/i.test(ua) ? 'Script' : 'Browser';
  const os = /Mac OS X/.test(ua) ? 'macOS' : /Windows/.test(ua) ? 'Windows' : /Linux/.test(ua) ? 'Linux' : /iPhone|iPad/.test(ua) ? 'iOS' : /Android/.test(ua) ? 'Android' : '';
  return os ? `${browser} on ${os}` : browser;
}

const METHOD_LABEL: Record<string, string> = { password: 'Password', saml: 'SAML SSO', oidc: 'OIDC SSO', invite: 'Login link' };

function SessionsCard() {
  const q = useQuery({ queryKey: keys.sessions, queryFn: listSessions });
  const revoke = useAuthMutation((id: string) => revokeSession(id));
  return (
    <section className="settings-card">
      <h2>Active sessions</h2>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} />
      ) : (
        <table className="settings-table">
          <tbody>
            {q.data.map((s) => (
              <tr key={s.id}>
                <td>
                  <span className="session-row">
                    <Laptop size={18} /> {browserLabel(s.userAgent)} {s.current && <span className="pill-chip small">This session</span>}
                  </span>
                </td>
                <td className="muted">{METHOD_LABEL[s.method] ?? s.method}</td>
                <td className="muted">{s.ip ?? '—'}</td>
                <td className="muted">Active {timeAgo(s.lastSeenAt)}</td>
                <td className="num">
                  {!s.current && (
                    <button className="btn btn-sm" onClick={() => revoke.mutate(s.id)} disabled={revoke.isPending}>
                      Sign out
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

/** Personal application keys (scripts, Terraform). The secret is shown once. */
export function AppKeysTab() {
  const q = useQuery({ queryKey: keys.appKeys, queryFn: listAppKeys });
  const [name, setName] = useState('');
  const [created, setCreated] = useState<string | null>(null);
  const create = useAuthMutation((n: string) => createAppKey(n));
  const revoke = useAuthMutation((id: string) => revokeAppKey(id));
  return (
    <div className="profile-cards">
      <section className="settings-card">
        <h2>Application Keys</h2>
        <p className="muted">
          A key acts as you on the ObserveCraft API: send it as <code>Authorization: Bearer &lt;key&gt;</code>. It has your role's permissions and stops working if your account is deactivated.
        </p>
        <form className="security-form" onSubmit={(e) => (e.preventDefault(), create.mutate(name, { onSuccess: (k) => (setCreated(k.key), setName('')) }))}>
          <input className="settings-input" placeholder="Key name (e.g. terraform-ci)" value={name} onChange={(e) => setName(e.target.value)} maxLength={100} required />
          <button className="btn btn-primary" disabled={create.isPending}>
            <KeyRound size={14} /> New Key
          </button>
        </form>
        {created && (
          <div className="new-key">
            <b>Copy this key now — it won't be shown again.</b>
            <div className="new-key-row">
              <code>{created}</code>
              <CopyButton text={created} />
              <button className="btn btn-sm" onClick={() => setCreated(null)}>
                Done
              </button>
            </div>
          </div>
        )}
        {(create.error || revoke.error) && <p className="form-error">{errorText(create.error ?? revoke.error)}</p>}
        {q.isPending ? (
          <Loading />
        ) : q.isError ? (
          <ErrorState error={q.error} />
        ) : q.data.length === 0 ? (
          <p className="muted">No application keys.</p>
        ) : (
          <table className="settings-table">
            <thead>
              <tr>
                <th>NAME</th>
                <th>KEY</th>
                <th>CREATED</th>
                <th>LAST USED</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {q.data.map((k) => (
                <tr key={k.id}>
                  <td>{k.name}</td>
                  <td className="mono">{k.prefix}…</td>
                  <td className="muted">{timeAgo(k.createdAt)}</td>
                  <td className="muted">{k.lastUsedAt ? timeAgo(k.lastUsedAt) : 'Never'}</td>
                  <td className="num">
                    <button className="icon-plain" title="Revoke" onClick={() => window.confirm(`Revoke "${k.name}"? Scripts using it will stop working.`) && revoke.mutate(k.id)}>
                      <Trash2 size={16} />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
