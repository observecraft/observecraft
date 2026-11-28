# Vanta

ObserveCraft sends an organization's compliance evidence to Vanta, the evidence Vanta's own Datadog integration reads
from Datadog. Vanta's Build Integrations API is push-based: ObserveCraft calls Vanta, Vanta never calls ObserveCraft,
so the install doesn't have to be reachable from the internet.

| Vanta's Datadog integration reads | ObserveCraft sends | Type (Developer Console base type) |
|---|---|---|
| Active users | The org's members | `user_account` (UserAccount) |
| Cloud Security vulnerabilities (libraries, hosts, containers) | Open vulnerabilities of hosts and container images (Cloud Security) and libraries (Code Security) | `vulnerable_component` (VulnerableComponent), then `package_vulnerability` (PackageVulnerability) |
| Cloud Security findings | Misconfigurations (the account scans' failed checks) and security signals (Cloud SIEM, App & API Protection, Workload Protection) | `security_task` (SecurityTask) |
| Infrastructure monitors | Monitors | `monitor` (CustomResource) |
| Cloud SIEM detection rules | Log detection rules | `detection_rule` (CustomResource) |
| Cloud SIEM log indexes | Log indexes | `log_index` (CustomResource) |
| Cloud SIEM notification rules, identity risks, attack paths | Not sent: ObserveCraft doesn't have them yet | |

## Set it up

1. In Vanta, **Settings → Developer Console**: create an app of type **Build Integrations**, distribution **Private**.
   Copy its client id and generate a client secret.
2. In the app, add a **resource** for each type to send, with the base type of the table above, and copy each
   resource's `resourceId`. The three custom resources are created with the schemas below (also on the page, with a
   copy button). Vulnerabilities need the vulnerable components too.
3. In ObserveCraft, **Integrations → Vanta** (an organization admin): the region (Vanta or Vanta Gov), the client id,
   the client secret and the `resourceId`s. **Test credentials**, then **Save and start syncing**.

The client secret is stored encrypted and never shown again; keeping the same client id keeps it. A type without a
`resourceId` isn't sent.

## How it syncs

- Every hour (and right after saving, or with **Sync now**), the server gets one token for the app (Vanta keeps one
  active token per app: a new one revokes the previous) and sends, for each configured type, **all** of its records,
  components before vulnerabilities. Vanta replaces what it had, so what is gone from ObserveCraft (a member removed, a
  vulnerability fixed or muted, a monitor deleted) is gone from Vanta at the next sync.
- Vanta takes up to 10,000 records of a type per request and 10 requests a minute per token. Vulnerabilities and
  findings are sent most severe first, so a cut keeps the important ones; the page says when records were left out.
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

## Vulnerabilities (`vulnerable_component`, `package_vulnerability`)

Open vulnerabilities only (muted and resolved ones aren't sent, so Vanta closes them):

- **Components**: hosts (`HOST`) and container images (`CONTAINER_REPOSITORY_IMAGE`) from Cloud Security; services
  (`OTHER`) and repositories (`CODE_REPOSITORY`) that use a vulnerable library, from Code Security. Each is sent once,
  with when it was last seen.
- **Vulnerabilities**: one per component that has it: the package and version, Vanta's severity (the CVSS score when the
  advisory has one, else the lower bound of its severity: critical 9, high 7, medium 4, low 0.1), the CVE when there is
  one (else the advisory id names it), the advisory's title, whether a fixed version exists and the version to upgrade to.

## Security findings (`security_task`)

Priority from the severity: critical `P0`, high `P1`, medium `P2`, low `P3`, else `P4`. Open ones are sent first, most
severe first; resolved ones are sent for 90 days, with when (`closedTimestamp`).

- **Misconfigurations**: Cloud Security's failed checks (the AWS account scans): `OPEN` while the check fails; `CLOSED`
  when it was muted or passes again. Labels: `cloud-security`, the provider, account, region, resource type and check
  (and `muted`). Checks that never failed aren't findings.
- **Security signals**: Cloud SIEM's, App & API Protection's and Workload Protection's signals: `OPEN` while open or under
  review, `CLOSED` once archived. Their ids are `signal:<id>`, linking to the signal; labels `security-signal`, the
  rule type, rule id, source and env. Suppressed signals aren't sent. Vanta's Datadog integration doesn't read signals:
  this gives alerts from logs and traces an SLA in Vanta too.

## Custom resources

Each is a resource of its own in the app (base type CustomResource), created with this schema (JSON Type Definition).
Vanta runs no test on custom resources by itself: write **Custom Tests** on their properties and map them to your
controls.

**Monitors** (`monitor`): every monitor but drafts.

```json
{
  "properties": {
    "monitorType": { "type": "string" },
    "muted": { "type": "boolean" },
    "notifies": { "type": "boolean" },
    "priority": { "type": "string" },
    "query": { "type": "string" },
    "recipients": { "type": "string" },
    "status": { "type": "string" },
    "tags": { "type": "string" }
  }
}
```

`priority` is `P1`–`P5` or `none`; `status` is `ok`, `warn`, `alert` or `nodata`; `recipients` lists the message's
`@` handles; `muted` is true while a downtime without a scope silences the whole monitor. Suggested tests: every monitor
notifies someone (`notifies` is true); no monitor stays muted (`muted` is false); critical monitors have a priority.

**Detection rules** (`detection_rule`): Cloud SIEM's log detection rules.

```json
{
  "properties": {
    "contentPack": { "type": "string" },
    "enabled": { "type": "boolean" },
    "isDefault": { "type": "boolean" },
    "notifies": { "type": "boolean" },
    "query": { "type": "string" },
    "severity": { "type": "string" },
    "tags": { "type": "string" }
  }
}
```

`severity` is the highest of the rule's cases (`info` to `critical`); `notifies` is true when a case notifies someone.
Suggested tests: rules are enabled; high and critical rules notify someone.

**Log indexes** (`log_index`):

```json
{
  "properties": {
    "dailyLimitMillions": { "type": "int32" },
    "exclusionFilters": { "type": "int32" },
    "filter": { "type": "string" },
    "flexRetentionDays": { "type": "int32" },
    "position": { "type": "int32" },
    "retentionDays": { "type": "int32" }
  }
}
```

`flexRetentionDays` and `dailyLimitMillions` (the daily limit in millions of events, rounded up) are 0 when there is
none; `exclusionFilters` counts the enabled ones. Custom properties are `string`, `int32`, `boolean` or `timestamp`
only. Suggested tests: logs are kept at least as long as your policy (for example `retentionDays + flexRetentionDays >= 90`).
