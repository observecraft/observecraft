import type { components } from '../generated/schema';

type S = components['schemas'];
export type AiGuardSettings = S['AiGuardSettings'];
export type AiGuardSummary = S['AiGuardSummary'];
export type AiGuardSeriesPoint = S['AiGuardSeriesPoint'];
export type AiGuardQuery = S['AiGuardQuery'];
export type AiGuardSearchRequest = S['AiGuardSearchRequest'];
export type AiGuardEvaluationPage = S['AiGuardEvaluationPage'];
export type AiGuardEvaluationRow = S['AiGuardEvaluationRow'];
export type AiGuardEvaluationDetail = S['AiGuardEvaluationDetail'];
export type AiGuardStep = S['AiGuardStep'];
export type AiGuardCategoryScore = S['AiGuardCategoryScore'];
export type AiGuardSdsFinding = S['AiGuardSdsFinding'];
export type AiGuardPlaygroundResult = S['AiGuardPlaygroundResult'];
export type AiGuardCategory = S['AiGuardCategory'];
export type AiDiscover = S['AiDiscover'];
export type AiProvider = S['AiProvider'];
export type AiService = S['AiService'];
export type TimeRange = { from: number; to: number };

/** A conversation message as the SDKs send it (OpenAI's chat shape). */
export interface AiGuardMessage {
  role: 'system' | 'developer' | 'user' | 'assistant' | 'tool';
  content?: string | { type: string; text?: string; image_url?: { url: string } }[] | null;
  tool_calls?: { id: string; function: { name: string; arguments: string } }[];
  tool_call_id?: string;
}
