import type { components } from '../generated/schema';

type S = components['schemas'];
export type AssistStatus = S['AssistStatus'];
export type AssistSkill = S['AssistSkill'];
export type AssistSettings = S['AssistSettings'];
export type ConversationSummary = S['ConversationSummary'];
export type ConversationView = S['ConversationView'];
export type ViewMessage = S['ViewMessage'];
export type ViewTool = S['ViewTool'];
export type AssistEvent = S['AssistEvent'];
export type AssistReport = S['AssistReport'];
export type AssistReportRequest = S['AssistReportRequest'];
export type ChatRequest = S['ChatRequest'];
export type AssistProviderSettings = S['AssistProviderSettings'];
export type AssistProviderView = S['AssistProviderView'];
export type AssistProviderUpdate = S['AssistProviderUpdate'];
export type AssistProviderTest = S['AssistProviderTest'];
export type AssistSurface = 'chat' | 'panel';
