// Dashboard reports data access. In "live" mode the Rust API stores, schedules and sends reports;
// otherwise the in-browser mock (mock/dashboardReports.ts) answers, with email "not configured".
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/dashboardReports';
import type { DashboardReport, DashboardReportDelivery, DashboardReportInput, DashboardReportOptions, DashboardReportPreview } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listReports = (): Promise<DashboardReport[]> => (live ? get('/api/v1/dashboard-reports', {}) : mock.listReports());
export const reportOptions = (): Promise<DashboardReportOptions> => (live ? get('/api/v1/dashboard-reports/options', {}) : mock.reportOptions());
export const createReport = (i: DashboardReportInput): Promise<DashboardReport> => (live ? post('/api/v1/dashboard-reports', i) : mock.createReport(i));
export const updateReport = (id: string, i: DashboardReportInput): Promise<DashboardReport> => (live ? put('/api/v1/dashboard-reports/{id}', { id }, i) : mock.updateReport(id, i));
export const deleteReport = (id: string): Promise<void> => (live ? del('/api/v1/dashboard-reports/{id}', { id }) : mock.deleteReport(id));
export const setReportSubscription = (id: string, subscribed: boolean): Promise<DashboardReport> =>
  live ? put('/api/v1/dashboard-reports/{id}/subscription', { id }, { subscribed }) : mock.setReportSubscription(id, subscribed);
export const previewReport = (id: string): Promise<DashboardReportPreview> => (live ? get('/api/v1/dashboard-reports/{id}/preview', { id }) : mock.previewReport(id));
export const sendReport = (id: string): Promise<DashboardReportDelivery[]> => (live ? postTo('/api/v1/dashboard-reports/{id}/send', { id }, undefined) : mock.sendReport(id));
