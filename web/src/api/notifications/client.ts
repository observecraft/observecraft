// Notification channels (@webhook-… / @slack-…) and the delivery log of monitor notifications.
import type { components } from '../generated/schema';
import { del, get, post } from '../http';

type S = components['schemas'];
export type NotificationChannel = S['NotificationChannel'];
export type Delivery = S['Delivery'];

export const listChannels = () => get('/api/v1/notification-channels', {});
export const createChannel = (c: { kind: 'webhook' | 'slack'; name: string; url: string; secret?: string | null }) => post('/api/v1/notification-channels', { ...c, secret: c.secret ?? null });
export const deleteChannel = (id: string) => del('/api/v1/notification-channels/{id}', { id });
export const listDeliveries = () => get('/api/v1/notification-deliveries', {});
