// Notification channels (@webhook-… / @slack-…) and the delivery log of monitor notifications.
import type { components } from '../generated/schema';
import { API_MODE, del, get, post } from '../http';
import { slackNotificationChannels } from '../mock/slack';

type S = components['schemas'];
export type NotificationChannel = S['NotificationChannel'];
export type Delivery = S['Delivery'];

/** Mock mode lists the mock Slack workspace's channels (Integrations → Slack). */
export const listChannels = (): Promise<NotificationChannel[]> => (API_MODE === 'live' ? get('/api/v1/notification-channels', {}) : Promise.resolve(slackNotificationChannels()));
export const createChannel = (c: { kind: 'webhook' | 'slack'; name: string; url: string; secret?: string | null }) => post('/api/v1/notification-channels', { ...c, secret: c.secret ?? null });
export const deleteChannel = (id: string) => del('/api/v1/notification-channels/{id}', { id });
export const listDeliveries = () => get('/api/v1/notification-deliveries', {});
