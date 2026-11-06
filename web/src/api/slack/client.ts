// Integrations → Slack. Outside live mode the in-browser mock (mock/slack.ts) answers.
import { API_MODE, del, get, patch, post, postTo } from '../http';
import * as mock from '../mock/slack';
import type { SlackAccount, SlackChannel, SlackConversation, SlackDisplay, SlackOverview, SlackTestDelivery } from './types';

export * from './types';

const live = API_MODE === 'live';

export const getSlackOverview = (): Promise<SlackOverview> => (live ? get('/api/v1/integrations/slack', {}) : mock.getSlackOverview());

/** Connects a workspace with its own Slack app's bot token (or reconnects the account with that name). */
export const connectSlackAccount = (input: { name: string; botToken: string; signingSecret?: string | null }): Promise<SlackAccount> =>
  live ? post('/api/v1/integrations/slack/accounts', { ...input, signingSecret: input.signingSecret || null }) : mock.connectSlackAccount(input);

/** "Add to Slack": the consent page URL of this install's Slack app. */
export const startSlackOAuth = (name: string): Promise<{ url: string }> => (live ? post('/api/v1/integrations/slack/oauth', { name }) : mock.startSlackOAuth(name));

export const disconnectSlackAccount = (id: string): Promise<void> => (live ? del('/api/v1/integrations/slack/accounts/{id}', { id }) : mock.disconnectSlackAccount(id));

export const listSlackConversations = (id: string): Promise<SlackConversation[]> => (live ? get('/api/v1/integrations/slack/accounts/{id}/conversations', { id }) : mock.listSlackConversations(id));

export const addSlackChannel = (accountId: string, name: string, display: SlackDisplay): Promise<SlackChannel> =>
  live ? postTo('/api/v1/integrations/slack/accounts/{id}/channels', { id: accountId }, { name, display }) : mock.addSlackChannel(accountId, name, display);

export const updateSlackChannel = (id: string, input: { name?: string; display?: SlackDisplay }): Promise<SlackChannel> =>
  live ? patch('/api/v1/integrations/slack/channels/{id}', { id }, { name: input.name ?? null, display: input.display ?? null }) : mock.updateSlackChannel(id, input);

export const removeSlackChannel = (id: string): Promise<void> => (live ? del('/api/v1/integrations/slack/channels/{id}', { id }) : mock.removeSlackChannel(id));

export const testSlackChannel = (id: string): Promise<SlackTestDelivery> => (live ? postTo('/api/v1/integrations/slack/channels/{id}/test', { id }, undefined) : mock.testSlackChannel(id));
