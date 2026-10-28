// Developer Portal → Scorecards: rules evaluated per service, grouped into scorecards, with levels. In
// "live" mode they come from the Rust API; otherwise the in-browser mock answers (mock/scorecards.ts).
//   GET    /api/v1/scorecards                                getScorecards
//   POST   /api/v1/scorecards/setup                          setupScorecards
//   POST   /api/v1/scorecards/evaluate                       evaluateScorecards
//   POST   /api/v1/scorecards/rules                          createScorecardRule
//   PUT    /api/v1/scorecards/rules/:id                      updateScorecardRule
//   DELETE /api/v1/scorecards/rules/:id                      deleteScorecardRule
//   POST   /api/v1/scorecards/rules/:id/outcomes             setScorecardOutcomes
//   PUT    /api/v1/scorecards/services/:service/metadata     updateServiceMetadata
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/scorecards';
import type { ScorecardOutcomesInput, ScorecardRuleInput, ScorecardRuleUpdate, ScorecardsOverview, ScorecardSetupInput, ServiceMetadataInput } from './types';

export * from './types';

const live = API_MODE === 'live';

export const getScorecards = (signal?: AbortSignal): Promise<ScorecardsOverview> => (live ? get('/api/v1/scorecards', {}, signal) : mock.getScorecards());
/** Creates the recommended scorecards with the chosen built-in rules enabled, and evaluates them. */
export const setupScorecards = (input: ScorecardSetupInput): Promise<ScorecardsOverview> => (live ? post('/api/v1/scorecards/setup', input) : mock.setupScorecards(input));
/** Reads telemetry again and evaluates every rule now. */
export const evaluateScorecards = (): Promise<ScorecardsOverview> => (live ? postTo('/api/v1/scorecards/evaluate', {}, undefined) : mock.evaluateScorecards());
export const createScorecardRule = (input: ScorecardRuleInput): Promise<ScorecardsOverview> => (live ? post('/api/v1/scorecards/rules', input) : mock.createScorecardRule(input));
export const updateScorecardRule = (id: string, input: ScorecardRuleUpdate): Promise<ScorecardsOverview> =>
  live ? put('/api/v1/scorecards/rules/{id}', { id }, input) : mock.updateScorecardRule(id, input);
export const deleteScorecardRule = (id: string): Promise<void> => (live ? del('/api/v1/scorecards/rules/{id}', { id }) : mock.deleteScorecardRule(id));
/** Reports outcomes of a custom rule per service. */
export const setScorecardOutcomes = (id: string, input: ScorecardOutcomesInput): Promise<ScorecardsOverview> =>
  live ? postTo('/api/v1/scorecards/rules/{id}/outcomes', { id }, input) : mock.setScorecardOutcomes(id, input);
export const updateServiceMetadata = (service: string, input: ServiceMetadataInput): Promise<ScorecardsOverview> =>
  live ? put('/api/v1/scorecards/services/{service}/metadata', { service }, input) : mock.updateServiceMetadata(service, input);
