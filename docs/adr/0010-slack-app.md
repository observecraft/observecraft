# ADR-0010: The ObserveCraft Slack app, controlled from Slack

Status: proposed (2026-11-24).

## Context

The Slack integration posts monitor alerts and On-Call pages to channels, and its one action is Mute (1h / 4h / 1d) on
an alert, a button that creates a downtime as the member who pressed it. Teams want to work from Slack: acknowledge
and resolve pages, mute and resolve monitors, declare and run incidents, and ask Assist questions, without opening
ObserveCraft first. Commercial observability products ship one Slack app that does this for every customer.

A Slack app sends every slash command, button press and event to one URL per kind. ObserveCraft is self-hosted: every
install has its own address, often reachable only on a private network. A single listed app would need a relay the
project hosts for every install, which a self-hosted product shouldn't depend on.

## Decision

**One app per install, publicly distributed.** Each operator creates the app from the manifest the install generates
(Integrations → Slack), turns on Slack's Public Distribution and sets `OC_SLACK_CLIENT_ID`, `OC_SLACK_CLIENT_SECRET`
and `OC_SLACK_SIGNING_SECRET` (Helm: the secret's optional `slack-client-id`, `slack-client-secret` and
`slack-signing-secret`); any workspace then installs it with "Add to Slack". A workspace may also connect its own app
with a bot token and signing secret; both kinds get the same features. No Marketplace listing and no relay; a listing
can be added later on top.

**Slack's address.** Slack must reach `/api/v1/public/slack/*` from the internet. When the install's UI is private,
only that path is published (for example on the intake's public ingress, with its rate limit) and `OC_SLACK_PUBLIC_URL`
(Helm `slack.publicUrl`) names it; the manifest's command, event, button and OAuth URLs use it, and link previews use
the UI's host (`OC_PUBLIC_URL`).

**Verification.** Every request is checked against the signing secret of an app connected to the request's team (the
workspace app's own, or the install's for OAuth installs), with a five-minute timestamp window against replays;
Slack's events URL check, which names no team, is accepted when any known app signed it. Anything else is 401.

**Who acts.** The Slack user's profile email (`users:read.email`) is matched to an active member of the organization the
workspace is connected to; that member's role decides what they may do, and actions are audited as them. A workspace
connected to several organizations the user belongs to asks once (buttons) and remembers the choice per Slack user
(Postgres `slack_user_orgs`); `/observecraft org` shows or changes it. Nothing in a request names the organization or
the member.

**Slack's three seconds.** Requests are acknowledged at once; work that calls Slack or the database at length runs
after the 200 and answers through the request's `response_url` (only Slack's own hooks host), `chat.postMessage` or
`chat.update`. Events API retries are recognized by `event_id` (Postgres `slack_events`, kept a day).

**Surface.** One command, `/observecraft <action>`, mentions of the bot, direct messages, buttons, modals and a message
shortcut, with our own names. The scopes of every planned feature are asked at install, so later features need no
reinstall: posting, channel lists, user emails, `commands`, `app_mentions:read`, `im:history`, `im:write`,
`channels:manage` (incident channels), `links:read`, `links:write` and `files:write` (link previews with images Slack
doesn't have to fetch from the install).

**Reuse.** Slack calls the same code as the UI (On-Call pages, incidents, downtimes, Assist runs with their read-only
tools); channels stay Datadog's `datadog_integration_slack_channel`. There is no parallel model for Slack.

## Consequences

- Operators create and own their Slack app; the setup guide (`docs/slack.md`) and the page's manifest make that a
  copy-paste step. Workspaces installed before a scope change reinstall once (the page lists missing scopes).
- Publishing `/api/v1/public/slack` exposes only signed endpoints; a stolen signing secret lets someone act as any
  member of a connected organization whose email they can put on a Slack profile in that workspace, so the secret is
  stored sealed and rotating it in Slack and ObserveCraft is the remedy.
- Matching by email trusts the Slack workspace's email addresses, as Mute already did; Slack verifies them.
- Features arrive in stages (`IMPLEMENTATION_PLAN.md`): the foundation, On-Call, monitors, incidents, then Assist and
  link previews.
