-- Authentication: passwords, MFA, sessions, invites / login links, application keys, SSO (SAML + OIDC)
-- configuration and an audit trail. Secrets that must be read back (TOTP seeds, OIDC client secrets)
-- are encrypted by the server (OC_SECRET_KEY); everything else that is only compared is hashed.

ALTER TABLE users
    ADD COLUMN password_hash   TEXT,
    -- AES-256-GCM(OC_SECRET_KEY) of the base32 TOTP seed, once MFA is enabled.
    ADD COLUMN mfa_secret_enc  TEXT,
    -- Seed being enrolled (not yet confirmed with a code).
    ADD COLUMN mfa_pending_enc TEXT,
    ADD COLUMN failed_logins   INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN locked_until    TIMESTAMPTZ,
    -- Stable subject from the identity provider (SAML NameID / OIDC sub), per provider.
    ADD COLUMN external_id     TEXT;
CREATE UNIQUE INDEX users_org_external ON users (org_id, external_id) WHERE external_id IS NOT NULL;
CREATE INDEX users_email ON users (lower(email));

-- Browser sessions. The cookie carries a random token; only its SHA-256 is stored.
CREATE TABLE sessions (
    token_hash   BYTEA PRIMARY KEY,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- password | saml | oidc | invite
    method       TEXT NOT NULL,
    -- True until the second factor is verified; such a session can only call /auth/mfa/verify.
    mfa_pending  BOOLEAN NOT NULL DEFAULT false,
    ip           TEXT,
    user_agent   TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX sessions_user ON sessions (user_id);

-- One-time links: invitations and admin-issued login (password reset) links.
CREATE TABLE login_tokens (
    token_hash BYTEA PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    purpose    TEXT NOT NULL CHECK (purpose IN ('invite', 'reset', 'bootstrap')),
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ
);

-- Application keys: act as their owner on the API (scripts, Terraform). Shown once; stored hashed.
CREATE TABLE app_keys (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    key_hash     BYTEA NOT NULL UNIQUE,
    -- First characters, to recognise a key in lists ("ocak_3f9a…").
    prefix       TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ
);
CREATE INDEX app_keys_user ON app_keys (user_id);

-- Per-org login methods and SSO configuration.
CREATE TABLE auth_settings (
    org_id               TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    password_enabled     BOOLEAN NOT NULL DEFAULT true,
    -- When true, only SSO may be used (admins keep password access as break-glass).
    sso_required         BOOLEAN NOT NULL DEFAULT false,
    -- Create users on their first SSO login (otherwise they must be invited first).
    jit_provisioning     BOOLEAN NOT NULL DEFAULT true,
    default_role_id      TEXT REFERENCES roles(id) ON DELETE SET NULL,
    saml_enabled         BOOLEAN NOT NULL DEFAULT false,
    saml_idp_metadata    TEXT,
    -- Attribute holding the user's email / display name / groups (Entra defaults).
    saml_email_attr      TEXT NOT NULL DEFAULT 'http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress',
    saml_name_attr       TEXT NOT NULL DEFAULT 'http://schemas.microsoft.com/identity/claims/displayname',
    saml_groups_attr     TEXT NOT NULL DEFAULT 'http://schemas.microsoft.com/ws/2008/06/identity/claims/groups',
    oidc_enabled         BOOLEAN NOT NULL DEFAULT false,
    oidc_issuer          TEXT,
    oidc_client_id       TEXT,
    oidc_client_secret_enc TEXT,
    oidc_groups_claim    TEXT NOT NULL DEFAULT 'groups',
    oidc_button_label    TEXT NOT NULL DEFAULT 'Sign in with SSO',
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO auth_settings (org_id, default_role_id) VALUES ('default', 'default-standard');

-- In-flight SSO logins (SAML AuthnRequest id / OIDC state+nonce+PKCE), single use, short-lived.
CREATE TABLE sso_requests (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    protocol      TEXT NOT NULL CHECK (protocol IN ('saml', 'oidc')),
    nonce         TEXT,
    pkce_verifier TEXT,
    redirect_to   TEXT NOT NULL DEFAULT '/',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- SAML assertion ids already consumed (replay protection), kept until they expire.
CREATE TABLE saml_seen_assertions (
    assertion_id TEXT PRIMARY KEY,
    expires_at   TIMESTAMPTZ NOT NULL
);

-- Security-relevant events: logins, failures, MFA and key changes, user/role/SSO administration.
CREATE TABLE audit_events (
    id         BIGSERIAL PRIMARY KEY,
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    actor_id   UUID REFERENCES users(id) ON DELETE SET NULL,
    actor      TEXT NOT NULL,
    action     TEXT NOT NULL,
    target     TEXT NOT NULL DEFAULT '',
    detail     TEXT NOT NULL DEFAULT '',
    ip         TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX audit_events_org ON audit_events (org_id, created_at DESC);
