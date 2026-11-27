# Vanta

ObserveCraft sends an organization's compliance evidence to Vanta, the evidence Vanta's own Datadog integration reads
from Datadog. Vanta's Build Integrations API is push-based: ObserveCraft calls Vanta, Vanta never calls ObserveCraft,
so the install doesn't have to be reachable from the internet.

| Vanta's Datadog integration reads | ObserveCraft sends | Vanta type | Status |
|---|---|---|---|
| Active users | The org's members | `user_account` (base type UserAccount) | Sent |
| Cloud Security vulnerabilities | Host, image and library vulnerabilities | `vulnerable_component`, `package_vulnerability` | Planned |
| Cloud Security findings | Misconfigurations | `security_task` | Planned |
| Infrastructure monitors | Monitors | custom resource | Planned |
| Cloud SIEM rules and log indexes | Detection rules, log indexes | custom resources | Planned |

## Set it up

1. In Vanta, **Settings → Developer Console**: create an app of type **Build Integrations**, distribution **Private**.
   Copy its client id and generate a client secret.
2. In the app, add a **resource** for each type to send, with its base type (Members: `UserAccount`), and copy each
   resource's `resourceId`.
3. In ObserveCraft, **Integrations → Vanta** (an organization admin): the region (Vanta or Vanta Gov), the client id,
   the client secret and the `resourceId`s. **Test credentials**, then **Save and start syncing**.

The client secret is stored encrypted and never shown again; keeping the same client id keeps it. A type without a
`resourceId` isn't sent.

## How it syncs

- Every hour (and right after saving, or with **Sync now**), the server gets one token for the app (Vanta keeps one
  active token per app: a new one revokes the previous) and sends, for each configured type, **all** of its records.
  Vanta replaces what it had, so a member removed from ObserveCraft disappears from Vanta at the next sync.
- Vanta takes up to 10,000 records of a type per request and 10 requests a minute per token; more than 10,000 are cut
  to the most important, with a note on the page.
- The page shows, per type, when it was last sent and how many records Vanta accepted and rejected, and the last sync's
  error (credentials refused, a `resourceId` of another app, throttling).

## Members (`user_account`)

| Field | From |
|---|---|
| `displayName`, `fullName` | The member's name (their email when they have none) |
| `accountName`, `email` | Their email |
| `permissionLevel` | `ADMIN` (Admin role), `EDITOR` (Standard), `BASE` (Read Only or custom roles only) |
| `status` | `ACTIVE`, or `DEACTIVATED` (with `deactivatedTimestamp`); pending invitations aren't sent |
| `authMethod` | `SSO` when they can only sign in through SAML or OIDC, `PASSWORD` otherwise |
| `mfaEnabled`, `mfaMethods` | Whether they set up an authenticator app (`OTP`), else `DISABLED` |
| `roleDescription` | Their roles |
| `createdTimestamp`, `lastLoginTimestamp` | When they joined and last signed in |
| `externalUrl` | Organization Settings → Users (Vanta requires `https`) |
