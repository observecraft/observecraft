import type { components } from '../generated/schema';

// The Slack integration's wire contract (routes/slack.rs): connected workspaces, the channels monitors
// mention as @slack-<account>-<channel> (the rows Terraform's datadog_integration_slack_channel manages
// too) and their display options.

type S = components['schemas'];
export type SlackOverview = S['SlackOverview'];
export type SlackAccount = S['SlackAccount'];
export type SlackChannel = S['SlackChannel'];
export type SlackConversation = S['SlackConversation'];
export type SlackDisplay = S['Display'];
export type SlackTestDelivery = S['TestDelivery'];

export const DEFAULT_DISPLAY: SlackDisplay = { message: true, notified: true, snapshot: true, tags: true, mute_buttons: false };

/** The handle monitors mention (`@slack-<account>-<channel>`), as the server computes it. */
export const slackHandle = (account: string, channel: string) => `slack-${account}-${channel.trim().replace(/^#+/, '').toLowerCase()}`;
