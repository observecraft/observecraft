# The ObserveCraft Slack app

ObserveCraft posts monitor alerts and On-Call pages to Slack, and teams act on them from Slack: `/observecraft`,
buttons on the messages and, as features arrive, incidents and questions to Assist (ADR-0010). Each install has its own
Slack app; this guide creates it.

## Before you start

Slack calls the install at `/api/v1/public/slack/*` (commands, buttons, events, OAuth). That path must be reachable
from the internet:

- If the install's UI is public (`OC_PUBLIC_URL`), there is nothing to do.
- If it isn't, publish only `/api/v1/public/slack` (for example on the intake's public ingress, with a rate limit) and
  set `OC_SLACK_PUBLIC_URL` (Helm `slack.publicUrl`) to that address, for example
  `https://observecraft-intake.example.com`. Every request on that path must carry a valid Slack signature; the rest of
  the API stays private.

Check from outside your network: `curl -X POST https://<address>/api/v1/public/slack/interactive` answers 401.

## Create the app

1. In ObserveCraft, open **Integrations → Slack → Set up** and copy the manifest (it already has this install's URLs).
2. On <https://api.slack.com/apps>, choose **Create New App → From a manifest**, pick the workspace, paste the manifest
   and create the app.
3. Under **Basic Information → Display Information**, upload the app icon (a square PNG between 512 and 2000 pixels,
   for example `web/public/logo.svg` rendered without its rounded corners) and set the background color.
4. **Install to Workspace** and allow.

## Connect a workspace

**With the app's token (one workspace).** In ObserveCraft, **Integrations → Slack → Connect**: an account name (the
`<account>` in `@slack-<account>-<channel>`), the **Bot User OAuth Token** (`xoxb-…`, OAuth & Permissions) and the
**Signing Secret** (Basic Information → App Credentials). The token and secret are stored sealed.

**With "Add to Slack" (any workspace, any organization of the install).** Turn on **Manage Distribution → Public
Distribution** in the app, then give the server the app's credentials: `OC_SLACK_CLIENT_ID`,
`OC_SLACK_CLIENT_SECRET` and `OC_SLACK_SIGNING_SECRET` (Helm: the keys `slack-client-id`, `slack-client-secret` and
`slack-signing-secret` of the chart's secret or `secrets.existingSecret`). Organizations then connect from
**Integrations → Slack → Add to Slack**.

## Use it

- Add channels to the account and mention them in monitors' messages: `@slack-<channel>` (or
  `@slack-<account>-<channel>` with several workspaces). Private channels need the bot invited
  (`/invite @ObserveCraft`).
- In Slack, `/observecraft help` lists the commands. People act as the ObserveCraft member with their Slack profile's
  email, with that member's role; someone who isn't a member is told so. When a workspace is connected to several
  organizations they belong to, ObserveCraft asks which one once; `/observecraft org` shows or changes it.

## Updating the app

When an ObserveCraft release changes the manifest (new scopes, events or commands), open the app's **App Manifest** on
api.slack.com, paste the manifest from Integrations → Slack again and save; Slack asks to reinstall when the scopes
changed. Workspaces installed with "Add to Slack" are listed with their missing scopes on the Slack page until they
reinstall.

Slack checks the **Event Subscriptions** URL when the manifest is saved: the install must already be running a release
with `/api/v1/public/slack/events`, and reachable, or Slack refuses the manifest.
