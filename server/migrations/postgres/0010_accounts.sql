-- Accounts and memberships: one sign-in identity (email, password, MFA) can belong to several orgs.
-- `users` stays the per-org membership row (roles, teams, status, SSO subject), so everything that
-- references users(id) keeps working; credentials move to `accounts`.

CREATE TABLE accounts (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Always stored lowercased.
    email             TEXT NOT NULL UNIQUE,
    name              TEXT NOT NULL,
    password_hash     TEXT,
    mfa_secret_enc    TEXT,
    mfa_pending_enc   TEXT,
    failed_logins     INTEGER NOT NULL DEFAULT 0,
    locked_until      TIMESTAMPTZ,
    -- Creates and manages organizations (granted from OC_INSTANCE_ADMIN_EMAIL at startup).
    is_instance_admin BOOLEAN NOT NULL DEFAULT false,
    -- The org a password sign-in lands in when it doesn't name one.
    last_org_id       TEXT REFERENCES orgs(id) ON DELETE SET NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at     TIMESTAMPTZ
);

-- One account per distinct email; when an email had users in several orgs, the one with a password
-- (then the most recently used) supplies the credentials.
INSERT INTO accounts (id, email, name, password_hash, mfa_secret_enc, mfa_pending_enc, failed_logins, locked_until, last_org_id, created_at, last_login_at)
SELECT DISTINCT ON (lower(email)) id, lower(email), name, password_hash, mfa_secret_enc, mfa_pending_enc, failed_logins, locked_until, org_id, created_at, last_login_at
FROM users
ORDER BY lower(email), (password_hash IS NOT NULL) DESC, last_login_at DESC NULLS LAST, created_at;

ALTER TABLE users ADD COLUMN account_id UUID REFERENCES accounts(id) ON DELETE CASCADE;
UPDATE users u SET account_id = a.id FROM accounts a WHERE a.email = lower(u.email);
ALTER TABLE users ALTER COLUMN account_id SET NOT NULL;
CREATE UNIQUE INDEX users_org_account ON users (org_id, account_id);
CREATE INDEX users_account ON users (account_id);
ALTER TABLE users
    DROP COLUMN password_hash,
    DROP COLUMN mfa_secret_enc,
    DROP COLUMN mfa_pending_enc,
    DROP COLUMN failed_logins,
    DROP COLUMN locked_until;

-- New memberships (invites, SSO just-in-time provisioning, seeds) join the account with their email,
-- creating it when the email is new.
CREATE FUNCTION users_link_account() RETURNS trigger AS $$
BEGIN
    IF NEW.account_id IS NULL THEN
        INSERT INTO accounts (email, name) VALUES (lower(NEW.email), NEW.name) ON CONFLICT (email) DO NOTHING;
        SELECT id INTO NEW.account_id FROM accounts WHERE email = lower(NEW.email);
    END IF;
    RETURN NEW;
END
$$ LANGUAGE plpgsql;
CREATE TRIGGER users_link_account BEFORE INSERT ON users FOR EACH ROW EXECUTE FUNCTION users_link_account();

-- A session belongs to an account and acts as one of its memberships (user_id / org_id), which
-- switching orgs changes.
ALTER TABLE sessions ADD COLUMN account_id UUID REFERENCES accounts(id) ON DELETE CASCADE;
UPDATE sessions s SET account_id = u.account_id FROM users u WHERE u.id = s.user_id;
DELETE FROM sessions WHERE account_id IS NULL;
ALTER TABLE sessions ALTER COLUMN account_id SET NOT NULL;
CREATE INDEX sessions_account ON sessions (account_id);

-- Organizations: suspended orgs reject every session and key.
ALTER TABLE orgs
    ADD COLUMN status     TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended')),
    ADD COLUMN created_by UUID REFERENCES accounts(id) ON DELETE SET NULL;

-- Instance-level actions (org created/suspended/deleted) are audited in the target org and here, so
-- they survive the org's deletion.
CREATE TABLE instance_audit_events (
    id         BIGSERIAL PRIMARY KEY,
    actor_id   UUID REFERENCES accounts(id) ON DELETE SET NULL,
    actor      TEXT NOT NULL,
    action     TEXT NOT NULL,
    target     TEXT NOT NULL DEFAULT '',
    detail     TEXT NOT NULL DEFAULT '',
    ip         TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
