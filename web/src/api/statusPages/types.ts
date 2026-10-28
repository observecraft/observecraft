import type { components } from '../generated/schema';

type S = components['schemas'];
export type ComponentStatus = S['ComponentStatus'];
export type NoticeStatus = S['NoticeStatus'];
export type StatusPageVisibility = S['StatusPageVisibility'];
export type StatusPageSummary = S['StatusPageSummary'];
export type StatusPageDetail = S['StatusPageDetail'];
export type StatusPageComponent = S['StatusPageComponent'];
export type StatusPageComponentInput = S['StatusPageComponentInput'];
export type StatusPageInput = S['StatusPageInput'];
export type StatusPageNotice = S['StatusPageNotice'];
export type NoticeImpact = S['NoticeImpact'];
export type NoticeUpdate = S['NoticeUpdate'];
export type NoticeInput = S['NoticeInput'];
export type NoticeUpdateInput = S['NoticeUpdateInput'];
export type PublicStatusPage = S['PublicStatusPage'];

/** Best to worst. */
export const COMPONENT_STATUSES: ComponentStatus[] = ['operational', 'degraded_performance', 'partial_outage', 'major_outage'];
export const COMPONENT_STATUS_LABEL: Record<ComponentStatus, string> = {
  operational: 'Operational',
  degraded_performance: 'Degraded Performance',
  partial_outage: 'Partial Outage',
  major_outage: 'Major Outage',
};
/** What the whole page says for its worst component. */
export const OVERALL_STATUS_LABEL: Record<ComponentStatus, string> = {
  operational: 'All Systems Operational',
  degraded_performance: 'Degraded Performance',
  partial_outage: 'Partial Outage',
  major_outage: 'Major Outage',
};

export const NOTICE_STATUSES: NoticeStatus[] = ['investigating', 'identified', 'monitoring', 'resolved'];
export const NOTICE_STATUS_LABEL: Record<NoticeStatus, string> = { investigating: 'Investigating', identified: 'Identified', monitoring: 'Monitoring', resolved: 'Resolved' };

export const VISIBILITY_LABEL: Record<StatusPageVisibility, string> = { public: 'Public', internal: 'Internal' };
