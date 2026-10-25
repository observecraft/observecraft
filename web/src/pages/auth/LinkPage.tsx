import { useEffect, useState } from 'react';
import { useParams } from 'react-router-dom';
import { acceptLink, linkInfo, verifyMfa } from '../../api/auth/client';
import { AuthFrame } from './LoginPage';
import './auth.css';

type Info = { email: string; name: string; orgName: string; purpose: string; existingAccount: boolean };

/** Invitation / password reset / first-admin link: choose a password (or, for an existing account, confirm it) and sign in. */
export function LinkPage() {
  const token = useParams().token ?? '';
  const [info, setInfo] = useState<Info | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [mfa, setMfa] = useState(false);
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    linkInfo(token)
      .then((i) => {
        setInfo(i);
        setName(i.name);
      })
      .catch((e: Error) => setError(e.message));
  }, [token]);

  const joining = !!info?.existingAccount;
  const title = info?.purpose === 'reset' ? 'Set a new password' : info?.purpose === 'bootstrap' ? 'Set up the administrator account' : joining ? `Join ${info?.orgName}` : 'Join ObserveCraft';
  const submit = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong');
    } finally {
      setBusy(false);
    }
  };

  return (
    <AuthFrame>
      {!info ? (
        <>
          <h1 className="auth-title small">Login link</h1>
          {error ? <p className="auth-error">{error}</p> : <p className="auth-hint">Checking your link…</p>}
          <a className="auth-link" href="/login">
            Go to sign-in
          </a>
        </>
      ) : mfa ? (
        <>
          <h1 className="auth-title small">Two-factor authentication</h1>
          <p className="auth-hint">{joining ? 'Almost there.' : 'Your password is set.'} Enter the code from your authenticator app to finish signing in.</p>
          <form className="auth-form" onSubmit={(e) => (e.preventDefault(), void submit(async () => (await verifyMfa(code), window.location.assign('/'))))}>
            <input className="auth-input auth-code" inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} required autoFocus />
            {error && <p className="auth-error">{error}</p>}
            <button className="auth-submit" disabled={busy}>
              Verify
            </button>
          </form>
        </>
      ) : (
        <>
          <h1 className="auth-title small">{title}</h1>
          <p className="auth-hint">
            {info.email} · {info.orgName}
          </p>
          {joining && <p className="auth-hint">You already have an ObserveCraft account. Enter its password to join this organization.</p>}
          <form
            className="auth-form"
            onSubmit={(e) => {
              e.preventDefault();
              if (!joining && password !== confirm) {
                setError("Passwords don't match");
                return;
              }
              void submit(async () => {
                const r = await acceptLink(token, password, info.purpose === 'reset' || joining ? undefined : name);
                if (r.mfaRequired) setMfa(true);
                else window.location.assign('/');
              });
            }}
          >
            {joining ? (
              <input
                className="auth-input"
                type="password"
                placeholder="Your current password"
                autoComplete="current-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                autoFocus
              />
            ) : (
              <>
                {info.purpose !== 'reset' && <input className="auth-input" placeholder="Full name" value={name} onChange={(e) => setName(e.target.value)} maxLength={120} required />}
                <input
                  className="auth-input"
                  type="password"
                  placeholder="Password (at least 12 characters)"
                  autoComplete="new-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  minLength={12}
                  required
                  autoFocus
                />
                <input className="auth-input" type="password" placeholder="Confirm password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} required />
              </>
            )}
            {error && <p className="auth-error">{error}</p>}
            <button className="auth-submit" disabled={busy}>
              {info.purpose === 'reset' ? 'Set password and log in' : joining ? 'Join and log in' : 'Create account and log in'}
            </button>
          </form>
        </>
      )}
    </AuthFrame>
  );
}
