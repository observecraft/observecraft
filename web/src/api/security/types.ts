import type { components } from '../generated/schema';

type S = components['schemas'];
export type SecurityRule = S['SecurityRule'];
export type SecurityRuleSummary = S['SecurityRuleSummary'];
export type SecurityRuleInput = S['SecurityRuleInput'];
export type RuleQuery = S['RuleQuery'];
export type RuleCase = S['RuleCase'];
export type RuleFilter = S['RuleFilter'];
export type Suppression = S['Suppression'];
export type SuppressionInput = S['SuppressionInput'];
export type SecuritySignal = S['SecuritySignal'];
export type SignalDetail = S['SignalDetail'];
export type SignalSearchRequest = S['SignalSearchRequest'];
export type SignalSearchPage = S['SignalSearchPage'];
export type SignalFacet = S['SignalFacet'];
export type SignalBucket = S['SignalBucket'];
export type SignalStateInput = S['SignalStateInput'];
export type SignalBulkInput = S['SignalBulkInput'];
export type ContentPackStatus = S['ContentPackStatus'];
export type SiemOverview = S['SiemOverview'];
export type SecurityOverview = S['SecurityOverview'];
export type SecurityInbox = S['SecurityInbox'];
export type InboxFinding = S['InboxFinding'];
export type FeedItem = S['FeedItem'];
export type Severity = 'info' | 'low' | 'medium' | 'high' | 'critical';
export const SEVERITIES: Severity[] = ['critical', 'high', 'medium', 'low', 'info'];
/** Entities of a signal: `{ip: {top, count}, user: …}`. */
export interface SignalEntity {
  top: string[];
  count: number;
}
