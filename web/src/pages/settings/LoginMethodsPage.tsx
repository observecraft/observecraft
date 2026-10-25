import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { AlertTriangle, Check, CheckCircle2, ChevronDown, ChevronRight, Copy } from 'lucide-react';
import { getLoginMethods, updateLoginMethods, type AuthSettingsUpdate, type AuthSettingsView } from '../../api/auth/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useRoles } from './queries';

function Copyable({ label, value }: { label: string; value: string }) {
  const [done, setDone] = useState(false);
  return (
    <div className="sp-row">
      <span className="sp-label">{label}</span>
      <code className="sp-value">{value}</code>
      <button
        type="button"
        className="icon-plain"
        title="Copy"
        onClick={() => {
          void navigator.clipboard?.writeText(value);
          setDone(true);
          setTimeout(() => setDone(false), 1200);
        }}
      >
        {done ? <Check size={15} /> : <Copy size={15} />}
      </button>
    </div>
  );
}

function Toggle({ checked, onChange, label, hint, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: string; hint?: string; disabled?: boolean }) {
  return (
    <label className={`lm-toggle${disabled ? ' disabled' : ''}`}>
      <input type="checkbox" className="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span>
        <b>{label}</b>
        {hint && <span className="muted">{hint}</span>}
      </span>
    </label>
  );
}

function toForm(v: AuthSettingsView): AuthSettingsUpdate {
  return {
    passwordEnabled: v.passwordEnabled,
    ssoRequired: v.ssoRequired,
    jitProvisioning: v.jitProvisioning,
    defaultRoleId: v.defaultRoleId ?? null,
    samlEnabled: v.saml.enabled,
    samlIdpMetadata: v.saml.idpMetadata ?? '',
    samlIdpMetadataUrl: '',
    samlEmailAttribute: v.saml.emailAttribute,
    samlNameAttribute: v.saml.nameAttribute,
    samlGroupsAttribute: v.saml.groupsAttribute,
    oidcEnabled: v.oidc.enabled,
    oidcIssuer: v.oidc.issuer ?? '',
    oidcClientId: v.oidc.clientId ?? '',
    oidcClientSecret: '',
    oidcGroupsClaim: v.oidc.groupsClaim,
    oidcButtonLabel: v.oidc.buttonLabel,
  };
}

export function LoginMethodsPage() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['auth', 'login-methods'], queryFn: getLoginMethods });
  const roles = useRoles();
  const [form, setForm] = useState<AuthSettingsUpdate | null>(null);
  const [metaMode, setMetaMode] = useState<'url' | 'xml'>('url');
  const [guide, setGuide] = useState(false);
  const save = useMutation({
    mutationFn: updateLoginMethods,
    onSuccess: (v) => {
      qc.setQueryData(['auth', 'login-methods'], v);
      setForm(toForm(v));
    },
  });

  useEffect(() => {
    if (q.data && !form) setForm(toForm(q.data));
  }, [q.data, form]);

  if (q.isPending || !form) return q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <Loading />;
  const v = save.data ?? q.data!;
  const set = (p: Partial<AuthSettingsUpdate>) => setForm({ ...form, ...p });
  const sp = v.serviceProvider;

  return (
    <div className="settings-pad lm-page">
      <div className="settings-title-row">
        <h1 className="settings-title">Login Methods</h1>
        <button className="btn btn-primary" disabled={save.isPending} onClick={() => save.mutate({ ...form, samlIdpMetadataUrl: metaMode === 'url' ? form.samlIdpMetadataUrl : '', samlIdpMetadata: metaMode === 'xml' ? form.samlIdpMetadata : null })}>
          {save.isPending ? 'Saving…' : 'Save'}
        </button>
      </div>
      {save.isError && (
        <p className="form-error lm-banner">
          <AlertTriangle size={15} /> {(save.error as Error).message}
        </p>
      )}
      {save.isSuccess && (
        <p className="form-ok lm-banner">
          <CheckCircle2 size={15} /> Saved. Changes apply to the next sign-in.
        </p>
      )}

      <section className="settings-card">
        <h2>Methods</h2>
        <Toggle checked={form.passwordEnabled} onChange={(passwordEnabled) => set({ passwordEnabled })} label="Email and password" hint="Users sign in with a password set from their invitation link. Optional authenticator-app MFA." />
        <Toggle checked={form.samlEnabled} onChange={(samlEnabled) => set({ samlEnabled })} label="SAML single sign-on" hint="Microsoft Entra ID, Okta, ADFS, Google Workspace, …" />
        <Toggle checked={form.oidcEnabled} onChange={(oidcEnabled) => set({ oidcEnabled })} label="OpenID Connect single sign-on" hint="Entra ID app registrations, Google, Okta, Auth0, Keycloak, …" />
        <hr className="lm-rule" />
        <Toggle
          checked={form.ssoRequired}
          disabled={!form.samlEnabled && !form.oidcEnabled}
          onChange={(ssoRequired) => set({ ssoRequired })}
          label="Require single sign-on"
          hint="Only SSO may be used. Admins keep password access as a break-glass in case the identity provider is down."
        />
        <Toggle checked={form.jitProvisioning} onChange={(jitProvisioning) => set({ jitProvisioning })} label="Create users on first SSO sign-in" hint="Otherwise people must be invited before they can sign in with SSO." />
        <label className="form-label lm-role">
          Default role for new SSO users
          <select className="settings-input" value={form.defaultRoleId ?? ''} onChange={(e) => set({ defaultRoleId: e.target.value || null })}>
            <option value="">No role</option>
            {(roles.data ?? []).map((r) => (
              <option key={r.id} value={r.id}>
                {r.name}
              </option>
            ))}
          </select>
          <span className="muted">Role mappings (SAML Group Mappings → Role Mappings) override this at each sign-in.</span>
        </label>
      </section>

      <section className="settings-card">
        <h2>
          SAML {v.saml.enabled && <span className="status-pill status-active">ENABLED</span>}
        </h2>
        <p className="muted">Give these values to your identity provider. ObserveCraft only accepts SHA-256 (or stronger) signed responses.</p>
        <div className="sp-box">
          <Copyable label="Identifier (Entity ID)" value={sp.entityId} />
          <Copyable label="Reply URL (ACS)" value={sp.acsUrl} />
          <Copyable label="SP metadata" value={sp.metadataUrl} />
        </div>
        <div className="lm-meta-switch">
          <span className="form-label">Identity provider metadata</span>
          <div className="apm-seg">
            <button className={metaMode === 'url' ? 'active' : ''} onClick={() => setMetaMode('url')}>
              Metadata URL
            </button>
            <button className={metaMode === 'xml' ? 'active' : ''} onClick={() => setMetaMode('xml')}>
              Paste XML
            </button>
          </div>
        </div>
        {metaMode === 'url' ? (
          <input
            className="settings-input wide"
            placeholder="https://login.microsoftonline.com/<tenant>/federationmetadata/2007-06/federationmetadata.xml?appid=<app id>"
            value={form.samlIdpMetadataUrl ?? ''}
            onChange={(e) => set({ samlIdpMetadataUrl: e.target.value })}
          />
        ) : (
          <textarea className="settings-input wide mono-area" rows={6} placeholder="<EntityDescriptor …>" value={form.samlIdpMetadata ?? ''} onChange={(e) => set({ samlIdpMetadata: e.target.value })} />
        )}
        {v.saml.idpEntityId && (
          <p className="lm-status ok">
            <CheckCircle2 size={15} /> Current IdP: <b>{v.saml.idpEntityId}</b> · SSO URL {v.saml.idpSsoUrl}
          </p>
        )}
        {v.saml.problem && (
          <p className="lm-status bad">
            <AlertTriangle size={15} /> {v.saml.problem}
          </p>
        )}
        <div className="lm-grid">
          <label className="form-label">
            Email attribute
            <input className="settings-input" value={form.samlEmailAttribute ?? ''} onChange={(e) => set({ samlEmailAttribute: e.target.value })} />
          </label>
          <label className="form-label">
            Name attribute
            <input className="settings-input" value={form.samlNameAttribute ?? ''} onChange={(e) => set({ samlNameAttribute: e.target.value })} />
          </label>
          <label className="form-label">
            Groups attribute
            <input className="settings-input" value={form.samlGroupsAttribute ?? ''} onChange={(e) => set({ samlGroupsAttribute: e.target.value })} />
          </label>
        </div>
      </section>

      <section className="settings-card">
        <h2>
          OpenID Connect {v.oidc.enabled && <span className="status-pill status-active">ENABLED</span>}
        </h2>
        <div className="sp-box">
          <Copyable label="Redirect URI" value={sp.oidcRedirectUrl} />
        </div>
        <div className="lm-grid">
          <label className="form-label">
            Issuer URL
            <input className="settings-input" placeholder="https://login.microsoftonline.com/<tenant>/v2.0" value={form.oidcIssuer ?? ''} onChange={(e) => set({ oidcIssuer: e.target.value })} />
          </label>
          <label className="form-label">
            Client ID
            <input className="settings-input" value={form.oidcClientId ?? ''} onChange={(e) => set({ oidcClientId: e.target.value })} />
          </label>
          <label className="form-label">
            Client secret
            <input
              className="settings-input"
              type="password"
              autoComplete="new-password"
              placeholder={v.oidc.hasClientSecret ? 'Stored — leave empty to keep' : ''}
              value={form.oidcClientSecret ?? ''}
              onChange={(e) => set({ oidcClientSecret: e.target.value })}
            />
          </label>
          <label className="form-label">
            Groups claim
            <input className="settings-input" value={form.oidcGroupsClaim ?? ''} onChange={(e) => set({ oidcGroupsClaim: e.target.value })} />
          </label>
          <label className="form-label">
            Button label
            <input className="settings-input" value={form.oidcButtonLabel ?? ''} onChange={(e) => set({ oidcButtonLabel: e.target.value })} />
          </label>
        </div>
      </section>

      <section className="settings-card">
        <button className="lm-guide-toggle" onClick={() => setGuide((g) => !g)}>
          {guide ? <ChevronDown size={18} /> : <ChevronRight size={18} />} Setting up Microsoft Entra ID
        </button>
        {guide && (
          <div className="lm-guide">
            <h4>SAML (Enterprise application)</h4>
            <ol>
              <li>Entra admin center → Enterprise applications → New application → Create your own (non-gallery) → Single sign-on → SAML.</li>
              <li>Basic SAML Configuration: Identifier = the Entity ID above; Reply URL = the ACS URL above.</li>
              <li>SAML Certificates: keep Signing Algorithm SHA-256; signing option "Sign SAML assertion" (or response and assertion).</li>
              <li>Attributes &amp; Claims → Add a group claim → "Groups assigned to the application" (avoids group overage), source attribute Group ID.</li>
              <li>Copy "App Federation Metadata Url" into Metadata URL above, enable SAML and save.</li>
              <li>Assign users or groups to the application, then map each Entra group object id under SAML Group Mappings (attribute key: the groups attribute above).</li>
            </ol>
            <h4>OpenID Connect (App registration)</h4>
            <ol>
              <li>App registrations → New registration; platform Web; redirect URI = the Redirect URI above.</li>
              <li>Certificates &amp; secrets → New client secret; copy it into Client secret.</li>
              <li>Token configuration → Add groups claim (ID token, Group ID). Issuer URL: https://login.microsoftonline.com/&lt;tenant-id&gt;/v2.0.</li>
              <li>Map group object ids under SAML Group Mappings with attribute key "groups" (or the Entra groups claim URI — both match).</li>
            </ol>
          </div>
        )}
      </section>
    </div>
  );
}
