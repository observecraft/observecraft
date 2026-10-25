-- `mfa_enabled` now mirrors real TOTP enrollment for password users (SSO users' MFA is the IdP's).
UPDATE users SET mfa_enabled = (mfa_secret_enc IS NOT NULL) WHERE password_hash IS NOT NULL;
