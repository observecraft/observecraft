// AI Guard (Security → AI Guard). In "live" mode from the Rust API, otherwise the in-browser mock
// (mock/aiGuard.ts).
//   POST /api/v1/ai-guard/summary|evaluations     Investigate: cards, charts and the evaluated steps
//   GET  /api/v1/ai-guard/evaluations/:id         one step with its run, scores and the SDK span
//   POST /api/v1/ai-guard/playground              evaluate pasted messages (not stored)
//   POST /api/v1/ai-guard/discover                AI services and model providers from LLM spans
//   GET|PUT /api/v1/ai-guard/settings             blocking, threshold, categories
//   GET  /api/v1/ai-guard/categories              attack categories
// Signals are security signals of the `ai_guard:true` rules (api/security).
import { API_MODE, get, post, put } from '../http';
import * as mock from '../mock/aiGuard';
import type {
  AiDiscover,
  AiGuardCategory,
  AiGuardEvaluationDetail,
  AiGuardEvaluationPage,
  AiGuardMessage,
  AiGuardPlaygroundResult,
  AiGuardQuery,
  AiGuardSearchRequest,
  AiGuardSettings,
  AiGuardSummary,
  TimeRange,
} from './types';

export * from './types';

const live = API_MODE === 'live';

export const getAiGuardSummary = (q: AiGuardQuery, signal?: AbortSignal): Promise<AiGuardSummary> => (live ? post('/api/v1/ai-guard/summary', q, signal) : mock.summary(q));
export const searchAiGuardEvaluations = (req: AiGuardSearchRequest, signal?: AbortSignal): Promise<AiGuardEvaluationPage> =>
  live ? post('/api/v1/ai-guard/evaluations', req, signal) : mock.evaluations(req);
export const getAiGuardEvaluation = (id: string): Promise<AiGuardEvaluationDetail> => (live ? get('/api/v1/ai-guard/evaluations/{id}', { id }) : mock.evaluation(id));
export const runAiGuardPlayground = (messages: AiGuardMessage[], meta: Record<string, unknown> = {}): Promise<AiGuardPlaygroundResult> =>
  live ? post('/api/v1/ai-guard/playground', { messages, meta } as never) : mock.playground(messages, meta);
export const getAiDiscover = (range: TimeRange): Promise<AiDiscover> => (live ? post('/api/v1/ai-guard/discover', { range }) : mock.discover(range));
export const getAiGuardSettings = (): Promise<AiGuardSettings> => (live ? get('/api/v1/ai-guard/settings', {}) : mock.settings());
export const updateAiGuardSettings = (s: AiGuardSettings): Promise<AiGuardSettings> => (live ? put('/api/v1/ai-guard/settings', {}, s) : mock.updateSettings(s));
export const listAiGuardCategories = (): Promise<AiGuardCategory[]> => (live ? get('/api/v1/ai-guard/categories', {}) : mock.categories());
