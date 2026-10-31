import type { components } from '../generated/schema';

type S = components['schemas'];
export type WpSummary = S['WpSummary'];
export type WpAgentCounts = S['WpAgentCounts'];
export type WpSeries = S['WpSeries'];
export type WpSeriesGroup = S['WpSeriesGroup'];
export type WpCount = S['WpCount'];
export type WpTactic = S['WpTactic'];
export type WpPack = S['WpPack'];
export type WpPolicyCard = S['WpPolicyCard'];
export type WpEventsRequest = S['WpEventsRequest'];
export type WpEventsPage = S['WpEventsPage'];
export type WpEvent = S['WpEvent'];
export type WpFacet = S['WpFacet'];
export type WpFindings = S['WpFindings'];
export type WpFinding = S['WpFinding'];
export type WpFindingRule = S['WpFindingRule'];
export type WpAgents = S['WpAgents'];
/** An Agent; `osLogo` / `platformLogo` are keys of BRAND_LOGOS (linux, windows, darwin / amazonecs, amazoneks, kubernetes). */
export type CsmAgent = S['CsmAgent'];
export type WpPolicy = S['WpPolicy'];
export type WpPolicyInput = S['WpPolicyInput'];
export type WpAgentRule = S['WpAgentRule'];
export type WpAgentRuleInput = S['WpAgentRuleInput'];
export type TimeRange = { from: number; to: number };
/** Event categories, as the summary and series key them. */
export type WpCategory = 'file' | 'process' | 'network' | 'kernel';
