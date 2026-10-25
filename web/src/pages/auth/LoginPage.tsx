import { useEffect, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowLeft, KeyRound } from 'lucide-react';
import { discoverSso, login, loginConfig, verifyMfa, type LoginConfig } from '../../api/auth/client';
import { LowPoly } from './LowPoly';
import './auth.css';

type Step = 'password' | 'sso' | 'mfa';

/** Same-app paths only (mirrors the server's open-redirect guard). */
export function safeRedirect(r: string | null): string {
  return r && r.startsWith('/') && !r.startsWith('//') && !r.startsWith('/login') ? r : '/';
}

export function AuthFrame({ children }: { children: React.ReactNode }) {
  return (
    <div className="auth-page">
      <LowPoly />
      <div className="auth-shell">
        <div className="auth-brand">
          <img src="/logo.svg" alt="" width={190} height={190} />
          <div className="auth-wordmark">OBSERVECRAFT</div>
        </div>
        <div className="auth-panel">{children}</div>
      </div>
      <footer className="auth-footer">ObserveCraft · Self-hosted observability</footer>
    </div>
  );
}

export function LoginPage() {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const redirect = safeRedirect(params.get('redirect'));
  // `/login?org=<id>` signs in to that organization (and offers its SSO); otherwise the account's last org.
  const org = params.get('org') ?? undefined;
  const [step, setStep] = useState<Step>('password');
  const [cfg, setCfg] = useState<LoginConfig | null>(null);
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [code, setCode] = useState('');
  const [error, setError] = useState<string | null>(params.get('error'));
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    loginConfig(org)
      .then(setCfg)
      .catch(() => setCfg(null));
  }, [org]);

  const done = () => {
    // Full reload so every cached query starts fresh as the new user.
    window.location.assign(redirect);
  };
  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Sign-in failed');
    } finally {
      setBusy(false);
    }
  };
  const ssoHref = (url: string) => `${url}?redirect=${encodeURIComponent(redirect)}`;
  const providers = [cfg?.oidc, cfg?.saml].filter(Boolean) as NonNullable<LoginConfig['oidc']>[];

  return (
    <AuthFrame>
      {step === 'password' && (
        <>
          <h1 className="auth-title">Log in to ObserveCraft</h1>
          {cfg && org && <p className="auth-org">{cfg.orgName}</p>}
          {providers.map((p) => (
            <a key={p.loginUrl} className="auth-sso-btn" href={ssoHref(p.loginUrl)}>
              <span className="auth-sso-icon">
                <KeyRound size={18} />
              </span>
              <span>{p.label}</span>
            </a>
          ))}
          <button className="auth-link" onClick={() => (setStep('sso'), setError(null))}>
            Using Single Sign-On?
          </button>
          {cfg?.passwordEnabled !== false && (
            <>
              <hr className="auth-rule" />
              <form
                className="auth-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  void run(async () => {
                    const r = await login(email, password, org);
                    if (r.mfaRequired) setStep('mfa');
                    else done();
                  });
                }}
              >
                <input className="auth-input" type="email" placeholder="Email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} required autoFocus />
                <input className="auth-input" type="password" placeholder="Password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} required />
                {error && <p className="auth-error">{error}</p>}
                <button className="auth-submit" disabled={busy}>
                  {busy ? 'Logging in…' : 'Log in'}
                </button>
              </form>
              <button className="auth-link" onClick={() => setError('Ask an ObserveCraft administrator for a login link (Organization Settings → Users → Login link).')}>
                Forgot password?
              </button>
            </>
          )}
          {cfg?.passwordEnabled === false && error && <p className="auth-error">{error}</p>}
        </>
      )}

      {step === 'sso' && (
        <>
          <h1 className="auth-title small">
            <button className="auth-back" onClick={() => (setStep('password'), setError(null))} aria-label="Back">
              <ArrowLeft size={18} />
            </button>
            Sign in with Your Organization
          </h1>
          <form
            className="auth-form"
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                const r = await discoverSso(email);
                if (!r.loginUrl) throw new Error('Your organization has not set up single sign-on. Log in with your password instead.');
                window.location.assign(ssoHref(r.loginUrl));
              });
            }}
          >
            <input className="auth-input" type="email" placeholder="Enter company email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} required autoFocus />
            {error && <p className="auth-error">{error}</p>}
            <button className="auth-submit" disabled={busy}>
              Next
            </button>
          </form>
        </>
      )}

      {step === 'mfa' && (
        <>
          <h1 className="auth-title small">Two-factor authentication</h1>
          <p className="auth-hint">Enter the 6-digit code from your authenticator app.</p>
          <form
            className="auth-form"
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                await verifyMfa(code);
                done();
              });
            }}
          >
            <input
              className="auth-input auth-code"
              inputMode="numeric"
              autoComplete="one-time-code"
              placeholder="123 456"
              maxLength={7}
              value={code}
              onChange={(e) => setCode(e.target.value.replace(/[^\d ]/g, ''))}
              required
              autoFocus
            />
            {error && <p className="auth-error">{error}</p>}
            <button className="auth-submit" disabled={busy}>
              Verify
            </button>
          </form>
          <button className="auth-link" onClick={() => navigate(0)}>
            Start over
          </button>
        </>
      )}
    </AuthFrame>
  );
}
