// Personal Settings beyond the profile (routes/personal.rs): preferences, email subscriptions and
// email verification. Outside live mode the in-browser mock (mock/personal.ts) answers.
//   GET/PUT /api/v1/me/preferences                     home page, time format, accessibility
//   GET     /api/v1/me/email-subscriptions             dashboard reports and monitors that email you
//   DELETE  /api/v1/me/email-subscriptions/reports/:id stop receiving a report
//   POST    /api/v1/me/verify-email                    send a verification link
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/personal';

export interface Preferences {
  homePage?: string | null;
  timeFormat: '12h' | '24h';
  weekStart: 'monday' | 'sunday';
  reducedMotion: boolean;
  highContrast: boolean;
  underlineLinks: boolean;
  colorBlindPalette: boolean;
}

export interface EmailSubscriptions {
  email: string;
  verified: boolean;
  emailEnabled: boolean;
  reports: { id: string; name: string; dashboardId: string; dashboardTitle: string; schedule: Record<string, unknown> }[];
  monitors: { id: number; name: string }[];
}

export const DEFAULT_PREFERENCES: Preferences = { homePage: null, timeFormat: '12h', weekStart: 'monday', reducedMotion: false, highContrast: false, underlineLinks: false, colorBlindPalette: false };

const live = API_MODE === 'live';

export const getPreferences = (): Promise<Preferences> => (live ? (get('/api/v1/me/preferences', {}) as Promise<Preferences>) : mock.getPreferences());
export const setPreferences = (p: Preferences): Promise<Preferences> =>
  live ? (put('/api/v1/me/preferences', {}, { ...p, homePage: p.homePage ?? null }) as Promise<Preferences>) : mock.setPreferences(p);
export const getEmailSubscriptions = (): Promise<EmailSubscriptions> => (live ? (get('/api/v1/me/email-subscriptions', {}) as Promise<EmailSubscriptions>) : mock.emailSubscriptions());
export const unsubscribeReport = (id: string): Promise<void> => (live ? del('/api/v1/me/email-subscriptions/reports/{id}', { id }) : mock.unsubscribeReport(id));
export const sendEmailVerification = (): Promise<{ email: string }> => (live ? post('/api/v1/me/verify-email', undefined as never) : mock.sendVerification());
