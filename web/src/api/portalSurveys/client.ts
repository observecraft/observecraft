// Developer Portal → Surveys. In "live" mode they come from the Rust API; otherwise the in-browser mock
// answers (mock/portalSurveys.ts).
//   GET    /api/v1/portal/surveys                      listSurveys
//   POST   /api/v1/portal/surveys                      createSurvey
//   GET    /api/v1/portal/surveys/:id                  getSurvey
//   PUT    /api/v1/portal/surveys/:id                  updateSurvey
//   DELETE /api/v1/portal/surveys/:id                  deleteSurvey
//   GET    /api/v1/portal/surveys/:id/form             getSurveyForm
//   POST   /api/v1/portal/surveys/:id/responses        submitSurveyResponse
//   GET    /api/v1/portal/surveys/:id/responses        listSurveyResponses
//   GET    /api/v1/portal/surveys/:id/results          getSurveyResults
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/portalSurveys';
import type { Survey, SurveyForm, SurveyInput, SurveyResponseInput, SurveyResponseRow, SurveyResults } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listSurveys = (signal?: AbortSignal): Promise<Survey[]> => (live ? get('/api/v1/portal/surveys', {}, signal) : mock.listSurveys());
export const getSurvey = (id: string, signal?: AbortSignal): Promise<Survey> => (live ? get('/api/v1/portal/surveys/{id}', { id }, signal) : mock.getSurvey(id));
export const createSurvey = (input: SurveyInput): Promise<Survey> => (live ? post('/api/v1/portal/surveys', input) : mock.createSurvey(input));
export const updateSurvey = (id: string, input: SurveyInput): Promise<Survey> => (live ? put('/api/v1/portal/surveys/{id}', { id }, input) : mock.updateSurvey(id, input));
export const deleteSurvey = (id: string): Promise<void> => (live ? del('/api/v1/portal/surveys/{id}', { id }) : mock.deleteSurvey(id));
/** The survey as a respondent sees it. */
export const getSurveyForm = (id: string, signal?: AbortSignal): Promise<SurveyForm> => (live ? get('/api/v1/portal/surveys/{id}/form', { id }, signal) : mock.getSurveyForm(id));
export const submitSurveyResponse = (id: string, input: SurveyResponseInput): Promise<void> =>
  live ? postTo('/api/v1/portal/surveys/{id}/responses', { id }, input) : mock.submitSurveyResponse(id, input);
export const getSurveyResults = (id: string, signal?: AbortSignal): Promise<SurveyResults> => (live ? get('/api/v1/portal/surveys/{id}/results', { id }, signal) : mock.getSurveyResults(id));
/** Every response, newest first (anonymous surveys name no one), for the CSV export. */
export const listSurveyResponses = (id: string, signal?: AbortSignal): Promise<SurveyResponseRow[]> =>
  live ? get('/api/v1/portal/surveys/{id}/responses', { id }, signal) : mock.listSurveyResponses(id);
