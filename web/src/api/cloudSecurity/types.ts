import type { components } from '../generated/schema';

type S = components['schemas'];
export type CloudSecuritySummary = S['CloudSecuritySummary'];
export type MisconfigFinding = S['MisconfigFinding'];
export type MisconfigRuleStatus = S['MisconfigRuleStatus'];
export type MisconfigPage = S['MisconfigPage'];
export type InfraVulnerability = S['InfraVulnerability'];
export type ResourceCategory = S['ResourceCategory'];
export type InfraPackage = S['InfraPackage'];
export type CvePage = S['CvePage'];
export type CveRow = S['CveRow'];
export type AdvisoryDetail = S['AdvisoryDetail'];
export interface CveQuery {
  query?: string;
  severity?: string;
  impact?: string;
  exploited?: boolean;
  offset?: number;
  limit?: number;
}
/** open (default) | muted | all */
export type FindingStatus = 'open' | 'muted' | 'all';
