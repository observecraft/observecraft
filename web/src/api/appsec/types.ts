import type { components } from '../generated/schema';

type S = components['schemas'];
export type SecurityTracePage = S['SecurityTracePage'];
export type SecurityTrace = S['SecurityTrace'];
export type WafTrigger = S['WafTrigger'];
export type AttackerPage = S['AttackerPage'];
export type Attacker = S['Attacker'];
export type AppsecUser = S['AppsecUser'];
export type ApiFindingsPage = S['ApiFindingsPage'];
export type ApiFinding = S['ApiFinding'];
export type FindingRuleStatus = S['FindingRuleStatus'];
export type EndpointPage = S['EndpointPage'];
export type ApiEndpointView = S['ApiEndpointView'];
export type AppsecService = S['AppsecService'];
export type AppsecOverview = S['AppsecOverview'];
export type TrendCount = S['TrendCount'];
export type NamedCount = S['NamedCount'];
export type BusinessLogic = S['BusinessLogic'];
export type DenylistView = S['DenylistView'];
export type DenylistInput = S['DenylistInput'];
export type WafPolicyView = S['WafPolicyView'];
export type TimeRange = { from: number; to: number };
/** custom_rule | exclusion_filter | policy */
export type WafKind = 'custom_rule' | 'exclusion_filter' | 'policy';
/** A WAF object as Datadog documents it. */
export interface WafDoc {
  id: string;
  type: string;
  attributes: Record<string, unknown> & { metadata?: { added_at?: string; added_by_name?: string; modified_at?: string; modified_by_name?: string } };
  meta?: Record<string, unknown>;
}
/** The OWASP API Security Top 10 2023. */
export const OWASP: [string, string][] = [
  ['API1', 'Broken Object Level Authorization'],
  ['API2', 'Broken Authentication'],
  ['API3', 'Broken Object Property Level Authorization'],
  ['API4', 'Unrestricted Resource Consumption'],
  ['API5', 'Broken Function Level Authorization'],
  ['API6', 'Unrestricted Access to Sensitive Business Flows'],
  ['API7', 'Server Side Request Forgery'],
  ['API8', 'Security Misconfiguration'],
  ['API9', 'Improper Inventory Management'],
  ['API10', 'Unsafe Consumption of APIs'],
];
