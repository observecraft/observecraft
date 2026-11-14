import type { components } from '../generated/schema';

type S = components['schemas'];
export type SdsConfiguration = S['SdsConfiguration'];
export type SdsGroup = S['SdsGroup'];
export type SdsGroupInput = S['SdsGroupInput'];
export type SdsRule = S['SdsRule'];
export type SdsRuleInput = S['SdsRuleInput'];
export type SdsReplacement = S['SdsReplacement'];
export type SdsSuppressions = S['SdsSuppressions'];
export type SdsStandardPattern = S['SdsStandardPattern'];
export type SdsTestRequest = S['SdsTestRequest'];
export type SdsTestResult = S['SdsTestResult'];

/** How a rule replaces what it finds (`SdsReplacement.kind`). */
export const REPLACEMENT_KINDS = ['none', 'hash', 'replacement_string', 'partial_replacement_from_beginning', 'partial_replacement_from_end'] as const;
export type ReplacementKind = (typeof REPLACEMENT_KINDS)[number];

export const REPLACEMENT_LABELS: Record<ReplacementKind, string> = {
  none: 'Tag only',
  hash: 'Hash',
  replacement_string: 'Replace with text',
  partial_replacement_from_beginning: 'Mask the first characters',
  partial_replacement_from_end: 'Mask the last characters',
};

export const CATEGORY_LABELS: Record<string, string> = { credentials: 'Credentials', finance: 'Finance', pii: 'Personal data', network: 'Network' };
