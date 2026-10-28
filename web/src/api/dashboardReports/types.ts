// Dashboard reports (Dashboards → Reports): a dashboard summarized over a time frame and sent on a
// schedule to member emails and notification channels. Wire types come from the server
// (server/crates/oc-api/src/routes/dashboard_reports.rs).
import type { components } from '../generated/schema';

type S = components['schemas'];

export type DashboardReport = S['DashboardReport'];
export type DashboardReportInput = S['DashboardReportInput'];
export type DashboardReportOptions = S['DashboardReportOptions'];
export type DashboardReportPreview = S['DashboardReportPreview'];
export type DashboardReportDelivery = S['DashboardReportDelivery'];
export type ReportSchedule = S['ReportSchedule'];
export type ReportWidgetSummary = S['ReportWidgetSummary'];

export type ReportFrequency = 'daily' | 'weekly' | 'monthly';
export const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

export const TIME_FRAMES: { key: string; label: string; ms: number }[] = [
  { key: '1h', label: 'Past 1 Hour', ms: 3_600_000 },
  { key: '4h', label: 'Past 4 Hours', ms: 4 * 3_600_000 },
  { key: '1d', label: 'Past 1 Day', ms: 86_400_000 },
  { key: '2d', label: 'Past 2 Days', ms: 2 * 86_400_000 },
  { key: '1w', label: 'Past 1 Week', ms: 7 * 86_400_000 },
  { key: '1mo', label: 'Past 1 Month', ms: 30 * 86_400_000 },
];

/** "Weekly on Monday at 09:00 (Europe/Paris)", as the server describes it. */
export function describeSchedule(s: ReportSchedule): string {
  const when = s.frequency === 'weekly' ? `Weekly on ${WEEKDAYS[s.weekday ?? 0] ?? 'Sunday'}` : s.frequency === 'monthly' ? `Monthly on day ${s.monthDay ?? 1}` : 'Daily';
  return `${when} at ${s.time} (${s.timezone.trim() || 'UTC'})`;
}

export const defaultSchedule = (): ReportSchedule => ({ frequency: 'weekly', time: '09:00', weekday: 1, monthDay: null, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC' });
