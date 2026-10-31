// In-browser stand-in for Personal Settings preferences and email subscriptions (routes/personal.rs).
import type { EmailSubscriptions, Preferences } from '../identity/personal';

const delay = <T>(v: T) => new Promise<T>((r) => setTimeout(() => r(structuredClone(v)), 120));

// Not imported from identity/personal (which imports this module): the same defaults.
let prefs: Preferences = { homePage: null, timeFormat: '12h', weekStart: 'monday', reducedMotion: false, highContrast: false, underlineLinks: false, colorBlindPalette: false };

const subs: EmailSubscriptions = {
  email: 'alex.morgan@example.com',
  verified: false,
  emailEnabled: true,
  reports: [
    { id: 'r1', name: 'Weekly storefront review', dashboardId: 'storefront-overview', dashboardTitle: 'Storefront Overview', schedule: { frequency: 'weekly', time: '09:00', weekday: 1 } },
    { id: 'r2', name: 'Daily checkout health', dashboardId: 'checkout-health', dashboardTitle: 'Checkout Health', schedule: { frequency: 'daily', time: '08:30' } },
  ],
  monitors: [
    { id: 1, name: 'High error rate on checkout' },
    { id: 4, name: 'Order queue backlog' },
  ],
};

export async function getPreferences(): Promise<Preferences> {
  return delay(prefs);
}

export async function setPreferences(p: Preferences): Promise<Preferences> {
  const home = p.homePage?.trim();
  if (home && (!home.startsWith('/') || home.startsWith('//'))) throw new Error('the home page is a path of this app, such as /apm');
  prefs = { ...p, homePage: home && home !== '/' ? home : null };
  return delay(prefs);
}

export async function emailSubscriptions(): Promise<EmailSubscriptions> {
  return delay(subs);
}

export async function unsubscribeReport(id: string): Promise<void> {
  subs.reports = subs.reports.filter((r) => r.id !== id);
  await delay(undefined);
}

export async function sendVerification(): Promise<{ email: string }> {
  return delay({ email: subs.email });
}
