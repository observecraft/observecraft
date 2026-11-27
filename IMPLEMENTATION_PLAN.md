# Implementation plan: the ObserveCraft Slack app (control from Slack)

Today the Slack integration posts monitor alerts and On-Call pages, and its only action is Mute (1h / 4h / 1d) on
an alert. The goal is a Slack app teams work from: On-Call, incidents and monitors controlled from Slack, and
questions to Assist answered in Slack, as Datadog's Slack app does for its products (functional reference only;
our own names, messages and layout).

Decisions:
- **One app per install, publicly distributed.** Slack sends every command, button and event of an app to one URL,
  and every ObserveCraft install has its own, so each operator creates the app from the manifest the install
  generates, turns on Slack's Public Distribution, and sets `OC_SLACK_CLIENT_ID` / `_CLIENT_SECRET` /
  `_SIGNING_SECRET`; any workspace then installs it with "Add to Slack" (OAuth). No Marketplace listing and no
  central relay (a hosted service the project would run); a listing can come later on top. Manual installs with a
  workspace's own app (bot token + signing secret) keep working with the same features.
- **Who acts is the Slack user's ObserveCraft membership.** The Slack profile's email (`users:read.email`) is matched
  to an active member of the workspace's organization, as Mute does today; actions run with that member's role
  (writes need Standard or Admin) and are audited as them. A workspace connected to several organizations asks the
  user once which one, remembered per Slack user. Nobody is identified from request input.
- **Slack's 3-second rule.** Commands, buttons and events are verified (signature, 5-minute timestamp window),
  acknowledged at once and handled in the background; answers go through `response_url`, `chat.postMessage` or
  `chat.update`. Event retries are deduplicated by `event_id`.
- **Messages are updated in place.** The messages the bot posts for pages, alerts and incidents are recorded
  (Postgres, per channel and `ts`), so a status change edits the card instead of posting a new one; details go in
  its thread.
- **One command:** `/observecraft <action>` (`help`, `oncall`, `monitors`, `incident`, `ask`), plus mentions of the
  bot, buttons, modals and a message shortcut. Our own names; nothing named after Datadog's products.
- **Reuse, not parallel models.** Slack calls the same code as the UI's handlers (incidents, On-Call pages,
  downtimes, Assist runs with their read-only tools); channels stay `datadog_integration_slack_channel`.
- **Scopes grow with the stages.** The page already lists missing scopes of OAuth installs and asks for a reinstall;
  manual installs update their app from the new manifest.

## Stage 1: The app's foundation
Goal: ADR-0010 and `docs/slack.md` (create, distribute and install the app); the manifest gains the slash command,
event subscriptions, the OAuth redirect URL and the new scopes; shared request verification and member resolution
(out of the Mute handler); `POST /api/v1/public/slack/commands` (`help`, and the organization choice when a
workspace has several) and `POST /api/v1/public/slack/events` (URL verification, deduplication); the Helm chart
takes the three `OC_SLACK_*` values from its secret; the Slack page shows the distribution setup.
Success Criteria: route tests with signed and unsigned requests (stale timestamps, wrong secret, unknown team,
retries), member resolution (non-member, viewer, several organizations), the manifest's content; `make test`; the
page live and mock.
Status: Complete (`slack_app.rs`: `/observecraft help|org`, events with URL check and `event_id` deduplication,
members by email with the organization choice in `slack_user_orgs`; 6 new route tests and the manifest parsed as YAML;
`OC_SLACK_PUBLIC_URL` / `OC_SLACK_HOOKS_URL`; Helm `slack.publicUrl` and the secret's `slack-*` keys; the page lists
every URL Slack needs)

## Stage 2: On-Call from Slack
Goal: pages are posted as cards with Acknowledge, Resolve and Reassign (a modal to pick the responder); the card is
updated in place as the page changes, wherever it changed (Slack, UI, email links); `/observecraft oncall [team]`
shows who is on call now and next.
Success Criteria: tests for the card per page state, each action through the On-Call handlers (roles, a page
already resolved), the in-place update and the command; a page acknowledged from Slack in a real workspace.
Status: Not Started

## Stage 3: Monitors from Slack
Goal: alert cards get Unmute (once muted) and Resolve beside Mute, and their recovery edits the alert card (thread
note) instead of a new message; `/observecraft monitors [query]` lists what is alerting, with the same buttons.
Success Criteria: tests for the buttons (roles, a downtime already removed), the card's states and the command's
search; muted, unmuted and resolved from Slack in a real workspace.
Status: Not Started

## Stage 4: Incidents from Slack
Goal: `/observecraft incident` opens a modal (title, severity, commander, services) and declares the incident; the
app creates its channel (`#inc-<id>-<slug>`, invites the declarer and commander) with a card for status, severity
and commander; changes made anywhere are posted to the channel; in the channel, `/observecraft incident status|
resolve` and the message shortcut "Add to incident timeline" (no history scope needed).
Success Criteria: tests for the modal's validation, declaring as the member, the channel name, the card, posting
changes and the shortcut; an incident run end to end from Slack in a real workspace.
Status: Not Started

## Stage 5: Assist in Slack and link previews
Goal: mentioning the bot or `/observecraft ask …` starts an Assist run as the member (read-only tools, their
permissions, the org's limits) and answers in the thread when it finishes; links to ObserveCraft (monitors,
dashboards and their widgets, incidents, log and trace searches) unfurl with a summary and, for graphs, an image
drawn by the email chart renderer and uploaded to Slack (never a URL Slack must reach).
Success Criteria: tests for starting the run from a mention and a command (non-member, limits), posting the answer,
each unfurl kind; a question answered and a dashboard link previewed in a real workspace.
Status: Not Started

# Implementation plan: Vanta (send compliance evidence)

Vanta's own Datadog integration reads Datadog's users and infrastructure monitors, Cloud Security's vulnerabilities
(libraries, hosts and containers) and findings, and Cloud SIEM's detection rules, log indexes and notification rules.
ObserveCraft sends the same evidence to Vanta through a private Build Integrations app (Vanta Developer Console →
Build Integrations → Private: client credentials, one `resourceId` per resource type). Vanta's API, not Datadog's:
Datadog has no Vanta push, so it is our own model (`docs/vanta.md`).

Decisions:
- **Per org, on the cloud crawlers' jobs.** Integrations → Vanta: the client id, the client secret (sealed), Vanta's
  region (`https://api.vanta.com` or `https://api.vanta-gov.com`, nothing else) and the `resourceId` of each type to
  send. One job per connection (provider `vanta`) every hour: Vanta keeps one active token per app (a new one revokes
  the previous), so one run gets a token and sends every configured type in turn.
- **Full state.** Each `PUT /v1/resources/<type>` carries all the resources of the type (Vanta deletes the ones left
  out), at most 10,000 (the most severe first, with a warning) and 10 MB; 10 requests a minute per token.
- **Status per type** (Postgres `vanta_sync_results`): sent, accepted, rejected, error, when.
- **Same evidence as Datadog's integration**, where ObserveCraft has it. Not yet: SIEM notification rules, identity
  risks and attack paths (not built).

## Stage 1: Connection and users
Goal: the connection (save, test with a token, remove, sync now), the hourly job, the per-type status, and members as
`user_account` (role → permission level, status, MFA, SSO or password, created and last login).
Success Criteria: tests for the mapping, the sync (one token, the request shape, accepted/rejected, refused token,
throttling, the 10,000 cap, a type without `resourceId` skipped), and the routes (secret sealed and never answered,
org isolation, the region allow-list, the job); `make test`; the page live and mock.
Status: Complete (17 Rust tests: API answers and errors, member mapping and query, the sync, the routes; the Vanta tile
as a native connector with Vanta's mark (site icon); Integrations → Vanta in mock mode, light and dark)

## Stage 2: Vulnerabilities
Goal: hosts, container images and repositories as `vulnerable_component`, then their CVEs as `package_vulnerability`
(Cloud Security's host and image vulnerabilities, Code Security's libraries), components first.
Status: Not Started

## Stage 3: Security findings
Goal: Cloud Security's misconfigurations as `security_task` (open or closed, priority from severity).
Status: Not Started

## Stage 4: Monitors and Cloud SIEM
Goal: monitors, detection rules and log indexes as custom resources; their schemas and suggested Vanta Custom Tests in
`docs/vanta.md`.
Status: Not Started
